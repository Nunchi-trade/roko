# 17 -- Authentication

> Four auth paths as a Pipeline of Verify Cells, followed by an Authorize Cell for workspace membership. Team workspace sharing with role-based access. Secret management with 3-tier priority. Device flow for headless machines. The auth pipeline fails closed -- every request passes through at least one Verify Cell and the Authorize Cell before reaching any route handler.

**Depends on**: [02-CELL](02-CELL.md) (Verify protocol), [03-GRAPH](03-GRAPH.md) (Pipeline Graph), [16-SECURITY](16-SECURITY.md) (capability intersection, fail-closed principle)

---

## 1. Overview

Authentication in Roko is expressed as a **Pipeline of Verify Cells**. The pipeline has two stages:

1. **Authentication** (4 Verify Cells): Each auth path is a Verify Cell that receives a request Signal and emits a Verdict: accept (with identity claims), reject (with reason), or skip (no matching credential). The authentication stage short-circuits on first acceptance -- if any Verify Cell accepts, the request proceeds to stage 2 with the identity from that Cell.

2. **Authorization** (1 Verify Cell): The `AuthorizeCell` receives the authenticated identity, checks workspace membership in `.roko/users/`, resolves the user's role, and verifies the role has grants for the requested route. It emits Accept (with identity enriched by role and grants) or Reject (with 403).

Four auth paths serve four surfaces:

| Path | Surface | Verify Cell | Credential |
|---|---|---|---|
| Privy/JWT | Dashboard (web) | `VerifyJwt` | JWT signed by Privy JWKS |
| API key | CLI, external integrations | `VerifyApiKey` | `sk_roko_...` header |
| Agent bearer token | Agents (relay, sidecar, inference) | `VerifyAgentToken` | `roko_agent_...` bearer |
| Relay auth | Feed subscribers (read path) | `VerifyRelayRead` | No credential (read-public) |

```
                                   STAGE 1: AUTHENTICATION
                     +---------+    +-----------+    +-------------+    +------------+
Request Signal ----> |VerifyJwt| -> |VerifyApiKey| -> |VerifyAgent  | -> |VerifyRelay | -+
                     |         |    |           |    |Token        |    |Read        |  |
                     +---------+    +-----------+    +-------------+    +------------+  |
                          |              |                  |                  |         |
                     accept/skip    accept/skip        accept/skip       accept/reject  |
                          |              |                  |                  |         |
                          +--------------+------------------+------------------+         |
                                         | (on accept)                                  |
                                         v                                              |
                                   STAGE 2: AUTHORIZATION                               |
                                  +-------------+                                       |
                                  | Authorize   | ---> Accept (identity + role + grants) |
                                  | Cell        | ---> Reject (403: not a member)        |
                                  +-------------+                                       |
                                         |                                              |
                                         v                                              |
                                   Route Handler                                        |
                                                                                        |
                          (all skip, no credential matched) <---------------------------+
                                         |
                                         v
                                    401 Unauthorized
```

If all four Verify Cells skip (no matching credential), the pipeline rejects with `401 Unauthorized`. Write operations that reach `VerifyRelayRead` without a prior acceptance are rejected -- relay read-public only applies to read operations.

---

## 2. Shared Types

All Verify Cells in the auth pipeline share these types for input, output, and identity representation.

```rust
/// Request Signal: the input to every Verify Cell in the auth pipeline.
/// Extracted from the HTTP request before entering the pipeline.
pub struct AuthRequest {
    pub method: String,
    pub path: String,
    pub headers: HashMap<String, String>,
    pub query: HashMap<String, String>,
}

/// Identity: the authenticated principal.
/// Produced by authentication Verify Cells, consumed by AuthorizeCell.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Identity {
    Privy {
        sub: String,
        email: Option<String>,
        wallet: Option<String>,
    },
    ApiKey {
        owner: String,
        scopes: Vec<ApiKeyScope>,
    },
    Agent {
        agent_id: AgentId,
    },
    RelayRead,
}

/// Verdict: the output of every Verify Cell.
/// Accept carries identity; Reject carries reason and HTTP status.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum VerdictKind {
    /// This Cell accepted the request. Identity is attached.
    Accept { identity: Identity },
    /// This Cell explicitly rejected the request.
    Reject { reason: String, status: u16 },
    /// This Cell does not handle this credential type. Pass to next Cell.
    Skip,
}

/// Enriched identity: the output of AuthorizeCell.
/// Contains the original identity plus workspace role and grants.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuthorizedIdentity {
    pub identity: Identity,
    pub role: WorkspaceRole,
    pub grants: Vec<RouteGrant>,
}

/// Workspace role: one of four levels.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum WorkspaceRole {
    Owner,
    Admin,
    Member,
    Viewer,
}

/// Route grant: a permission that a role has.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RouteGrant {
    pub methods: Vec<String>,   // e.g., ["GET", "POST", "PUT", "DELETE"] or ["*"]
    pub path_pattern: String,   // e.g., "/api/agents/*" or "*"
}
```

---

## 3. Auth Path 1: Privy/JWT (Dashboard Users)

```
Browser -> Privy SDK -> JWT -> roko-serve validates signature
```

Privy handles login (email, social, wallet). The dashboard includes the JWT in every API call. roko-serve validates the JWT signature against Privy's JWKS endpoint.

### 3.1 VerifyJwt Cell

```rust
pub struct VerifyJwt {
    /// Cached JWKS keys from Privy.
    jwks_cache: RwLock<JwksCache>,
    /// Privy app ID (public, baked into deployment).
    app_id: String,
}

struct JwksCache {
    keys: Vec<Jwk>,
    fetched_at: Instant,
    stale_keys: Option<Vec<Jwk>>,  // kept for stale-while-revalidate
}

impl Cell for VerifyJwt {
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Verify] }
    fn name(&self) -> &str { "verify-jwt" }

    fn input_schema(&self) -> TypeSchema {
        TypeSchema::named("AuthRequest")
    }
    fn output_schema(&self) -> TypeSchema {
        TypeSchema::named("Verdict<Identity::Privy>")
    }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let request = extract_request(&input)?;
        let token = match request.header("Authorization") {
            Some(h) if h.starts_with("Bearer ey") => &h[7..],
            _ => return Ok(vec![Signal::verdict(VerdictKind::Skip)]),
        };

        let keys = self.get_or_refresh_jwks().await?;
        match validate_jwt(token, &keys, &self.app_id) {
            Ok(claims) => Ok(vec![Signal::verdict(VerdictKind::Accept {
                identity: Identity::Privy {
                    sub: claims.sub,
                    email: claims.email,
                    wallet: claims.linked_accounts.wallet(),
                },
            })]),
            Err(ValidationError::KeyMismatch) => {
                // Key rotation: refetch JWKS once
                let fresh_keys = self.force_refresh_jwks().await?;
                match validate_jwt(token, &fresh_keys, &self.app_id) {
                    Ok(claims) => Ok(vec![Signal::verdict(VerdictKind::Accept {
                        identity: Identity::Privy {
                            sub: claims.sub,
                            email: claims.email,
                            wallet: claims.linked_accounts.wallet(),
                        },
                    })]),
                    Err(_) => Ok(vec![Signal::verdict(VerdictKind::Reject {
                        reason: "JWT signature invalid after JWKS refresh".into(),
                        status: 401,
                    })]),
                }
            }
            Err(e) => Ok(vec![Signal::verdict(VerdictKind::Reject {
                reason: format!("JWT validation failed: {e}"),
                status: 401,
            })]),
        }
    }
}
```

### 3.2 JWKS Caching Strategy

```
JWT arrives for validation
         |
         v
Check in-memory JWKS cache
         |
    cache hit ----------------> Validate JWT with cached keys
    (< 1 hour old)                    |
         |                       valid --> accept
    cache miss                        |
    (or expired)                 invalid --> refetch JWKS once (key rotation)
         |                                        |
         v                                   valid --> accept
Fetch GET https://auth.privy.io/                  |
      /.well-known/jwks.json              still invalid --> return 401
         |
    success --> update cache, validate JWT
         |
    failure --> use stale cache if available (stale-while-revalidate)
         |           |
         |      no stale cache --> return 401
         |
         v
    Log warning if cache is > 24 hours old:
    "[WARN] JWKS cache stale (26h). Privy endpoint may be down."
```

- **Cache TTL**: 1 hour. After 1 hour, the next validation triggers a background refetch.
- **Key rotation handling**: When a JWT fails validation against cached keys, refetch JWKS once before returning 401. Privy rotates keys periodically; the single retry catches rotation without adding latency to every request.
- **Endpoint unavailability**: If the JWKS endpoint is down, use stale cached keys. This is safe because Privy key rotation is infrequent (weeks or months between rotations). Log a warning if the cache exceeds 24 hours of staleness.
- **Startup**: On server start, fetch JWKS eagerly. If the fetch fails, the server starts but JWT validation returns 401 until the cache is populated. API key auth and agent token auth are unaffected.

### 3.3 Key Constraint: No PRIVY_APP_SECRET on User Deployments

Nunchi owns the Privy app. Users deploying their own roko instances don't have access to Privy's dashboard or app secret. Therefore:

- Privy's allowlist is NOT used. Privy login is open -- anyone can authenticate.
- Roko-serve handles all authorization. Each roko instance has its own user table in `.roko/users/`.
- No `PRIVY_APP_SECRET` needed on user deployments. JWT verification uses Privy's public JWKS endpoint.

```
Nunchi (company)                    User's roko instance (Railway)
----------------                    ------------------------------
Owns Privy app                      Has PRIVY_APP_ID (public, baked in)
  open login (no allowlist)         Does NOT have PRIVY_APP_SECRET
  anyone can authenticate           Verifies JWTs via public JWKS (no secret)

                                    .roko/users/
                                    +-- owner: will@nunchi.dev (first login)
                                    +-- invited: sarah@example.com -> member
                                    +-- (anyone else) -> 403 "Not a member"
```

Privy also provides an embedded wallet for chain interactions (signing transactions, delegating to agents). Optional -- users who don't need chain features never see wallet UI.

---

## 4. Auth Path 2: API Keys (CLI and External Integrations)

```bash
# Generate an API key (from dashboard or CLI)
roko config secrets set api.my-key
# -> sk_roko_aBcDeFgHiJkLmNoP

# Use it
roko status --server https://my-roko.up.railway.app --api-key sk_roko_...

# Or: roko login (browser-based)
roko login https://my-roko.up.railway.app
# -> Opens browser for Privy auth
# -> Stores session token in OS keychain

# On headless machines: device flow
roko login https://my-roko.up.railway.app
# -> Visit https://my-roko.up.railway.app/auth/device
#    Enter code: ABCD-EFGH
# -> Polls until approved
# -> Token stored in OS keychain
```

### 4.1 Four API Key Scopes

```rust
pub enum ApiKeyScope {
    Read,        // GET endpoints only
    AgentWrite,  // Agent CRUD + messaging
    PlanWrite,   // Plan/PRD creation and execution
    Admin,       // Everything including secrets and config
}
```

### 4.2 Scope-to-Route Mapping

```
Scope          Allowed methods    Allowed routes
-----          ---------------    --------------
read           GET                Any route

agent:write    POST, PUT, DELETE  /api/agents/*
               POST               /api/agents/*/message
               POST               /api/agents/*/token

plan:write     POST, PUT, DELETE  /api/plans/*
               POST               /api/plans/*/run
               POST, PUT, DELETE  /api/prd/*

admin          *                  * (all routes, including:)
                                  /api/api-keys/*
                                  /api/config/*
                                  /api/secrets/*
                                  /api/gateway/*
```

A key with multiple scopes has the union of their permissions. For example, a key with `[read, agent:write]` can GET any route and POST/PUT/DELETE on agent routes, but cannot touch plans or config.

### 4.3 Insufficient Scope Response

```json
HTTP 403 Forbidden

{
  "error": "insufficient_scope",
  "required": "agent:write",
  "has": "read",
  "route": "POST /api/agents/coder-1/message"
}
```

The response tells the caller exactly what scope they need, what they have, and which route triggered the rejection.

### 4.4 VerifyApiKey Cell

```rust
pub struct VerifyApiKey {
    /// Hash map of SHA-256(key) -> (scopes, owner, created_at).
    keys: RwLock<HashMap<[u8; 32], ApiKeyRecord>>,
}

impl Cell for VerifyApiKey {
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Verify] }
    fn name(&self) -> &str { "verify-api-key" }

    fn input_schema(&self) -> TypeSchema {
        TypeSchema::named("AuthRequest")
    }
    fn output_schema(&self) -> TypeSchema {
        TypeSchema::named("Verdict<Identity::ApiKey>")
    }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let request = extract_request(&input)?;
        let key = match request.header("X-Api-Key").or(request.header("Authorization")) {
            Some(k) if k.starts_with("sk_roko_") => k,
            Some(k) if k.starts_with("Bearer sk_roko_") => &k[7..],
            _ => return Ok(vec![Signal::verdict(VerdictKind::Skip)]),
        };

        let hash = sha256(key.as_bytes());
        let keys = self.keys.read();
        match keys.get(&hash) {
            Some(record) => {
                let required_scope = route_to_scope(&request.method, &request.path);
                if record.scopes.contains(&required_scope) || record.scopes.contains(&ApiKeyScope::Admin) {
                    Ok(vec![Signal::verdict(VerdictKind::Accept {
                        identity: Identity::ApiKey {
                            owner: record.owner.clone(),
                            scopes: record.scopes.clone(),
                        },
                    })])
                } else {
                    Ok(vec![Signal::verdict(VerdictKind::Reject {
                        reason: format!(
                            "insufficient_scope: required={:?}, has={:?}, route={} {}",
                            required_scope, record.scopes, request.method, request.path
                        ),
                        status: 403,
                    })])
                }
            }
            None => Ok(vec![Signal::verdict(VerdictKind::Reject {
                reason: "Invalid API key".into(),
                status: 401,
            })]),
        }
    }
}
```

---

## 5. Auth Path 3: Agent Bearer Tokens

Agents authenticate to the relay and to the inference proxy using bearer tokens issued by the control plane.

```bash
# Control plane issues token for an agent
POST /api/agents/:id/token
-> { "token": "roko_agent_...", "expires_at": "2026-04-25T00:00:00Z" }

# Agent uses token for relay connection
WS wss://relay.nunchi.dev/relay/ws
-> First message: { "type": "auth", "token": "roko_agent_..." }

# Agent uses token for inference proxy
POST /api/inference/proxy
Authorization: Bearer roko_agent_...
```

### 5.1 Token Storage: SHA-256 Hashed

Tokens are SHA-256 hashed before storage. The plaintext is returned exactly once at issuance.

### 5.2 Token Lifecycle

```
Agent created                Token issued                 30 days later
(POST /api/agents)           (response includes token)    (token expires)
       |                            |                            |
       v                            v                            v
  agent record              token = "roko_agent_..."      agent gets 401
  stored in DB              SHA-256 hash stored           on next request
                            plaintext returned ONCE
                                                                 |
                                                                 v
                                                          request new token
                                                          via relay or API
```

- **Issuance**: Tokens are issued when an agent is created (`POST /api/agents`). The response body includes the `token` field with the plaintext. This is the only time the plaintext is available.
- **Expiry**: Tokens expire after 30 days by default. Configurable per agent via `token_ttl_days` in `roko.toml`.
- **Revocation**: `DELETE /api/agents/{id}/token` immediately invalidates the token. The SHA-256 hash is removed from the valid token set. The agent receives `401 Unauthorized` on its next request.
- **Rotation with 5-minute grace period**: To rotate without downtime, issue a new token (`POST /api/agents/{id}/token`) before revoking the old one. During rotation, both the old and new tokens are valid for a 5-minute grace period. After 5 minutes, the old token is automatically invalidated.
- **Re-issuance**: An agent that receives 401 (expired or revoked token) should request a new token through the relay control channel or by calling `POST /api/agents/{id}/token` with admin-scoped auth.

### 5.3 VerifyAgentToken Cell

```rust
pub struct VerifyAgentToken {
    /// Hash map of SHA-256(token) -> (agent_id, expires_at, created_at).
    tokens: RwLock<HashMap<[u8; 32], AgentTokenRecord>>,
    /// Tokens in grace period: old_hash -> (grace_expires_at, agent_id).
    /// Stores the real agent_id so grace-period acceptance returns the
    /// correct identity, not a placeholder.
    grace_period: RwLock<HashMap<[u8; 32], (Instant, AgentId)>>,
}

struct AgentTokenRecord {
    agent_id: AgentId,
    expires_at: DateTime<Utc>,
    created_at: DateTime<Utc>,
}

impl Cell for VerifyAgentToken {
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Verify] }
    fn name(&self) -> &str { "verify-agent-token" }

    fn input_schema(&self) -> TypeSchema {
        TypeSchema::named("AuthRequest")
    }
    fn output_schema(&self) -> TypeSchema {
        TypeSchema::named("Verdict<Identity::Agent>")
    }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let request = extract_request(&input)?;
        let token = match extract_bearer(&request) {
            Some(t) if t.starts_with("roko_agent_") => t,
            _ => return Ok(vec![Signal::verdict(VerdictKind::Skip)]),
        };

        let hash = sha256(token.as_bytes());
        let tokens = self.tokens.read();

        // Check active tokens
        if let Some(record) = tokens.get(&hash) {
            if record.expires_at > Utc::now() {
                return Ok(vec![Signal::verdict(VerdictKind::Accept {
                    identity: Identity::Agent {
                        agent_id: record.agent_id.clone(),
                    },
                })]);
            } else {
                return Ok(vec![Signal::verdict(VerdictKind::Reject {
                    reason: "Agent token expired".into(),
                    status: 401,
                })]);
            }
        }

        // Check grace period tokens -- returns the real agent_id
        let grace = self.grace_period.read();
        if let Some((grace_until, agent_id)) = grace.get(&hash) {
            if *grace_until > Instant::now() {
                // Still in grace period -- accept with real identity, log deprecation
                ctx.bus.publish("auth:grace-period", Pulse::new(json!({
                    "agent_id": agent_id,
                    "grace_until": format!("{:?}", grace_until),
                    "warning": "old token in grace period, rotate soon",
                })));
                return Ok(vec![Signal::verdict(VerdictKind::Accept {
                    identity: Identity::Agent {
                        agent_id: agent_id.clone(),
                    },
                })]);
            }
        }

        Ok(vec![Signal::verdict(VerdictKind::Reject {
            reason: "Invalid agent token".into(),
            status: 401,
        })])
    }
}
```

### 5.4 Grace Period Token Rotation

When a new token is issued for an agent, the old token is moved to the grace period map with the real agent_id preserved.

```rust
impl VerifyAgentToken {
    /// Issue a new token, moving the old one to grace period.
    pub fn rotate_token(&self, agent_id: &AgentId) -> String {
        let new_plaintext = generate_agent_token();
        let new_hash = sha256(new_plaintext.as_bytes());

        let mut tokens = self.tokens.write();
        let mut grace = self.grace_period.write();

        // Find and move old token to grace period
        let old_hash = tokens.iter()
            .find(|(_, rec)| rec.agent_id == *agent_id)
            .map(|(hash, _)| *hash);

        if let Some(old) = old_hash {
            tokens.remove(&old);
            grace.insert(old, (
                Instant::now() + Duration::from_secs(300), // 5-minute grace
                agent_id.clone(),
            ));
        }

        // Insert new token
        tokens.insert(new_hash, AgentTokenRecord {
            agent_id: agent_id.clone(),
            expires_at: Utc::now() + chrono::Duration::days(30),
            created_at: Utc::now(),
        });

        new_plaintext
    }
}
```

---

## 6. Auth Path 4: Relay Auth

```
Read operations (subscribe, list feeds):    No auth required
Write operations (publish, register feed):  Require agent token
Admin operations (force-disconnect):        Require API key with admin scope
```

The dashboard can subscribe to presence and feeds without authentication. It needs auth only to send messages to agents or modify configuration.

### 6.1 VerifyRelayRead Cell

```rust
pub struct VerifyRelayRead;

impl Cell for VerifyRelayRead {
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Verify] }
    fn name(&self) -> &str { "verify-relay-read" }

    fn input_schema(&self) -> TypeSchema {
        TypeSchema::named("AuthRequest")
    }
    fn output_schema(&self) -> TypeSchema {
        TypeSchema::named("Verdict<Identity::RelayRead>")
    }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let request = extract_request(&input)?;

        // Read-public: allow GET and WebSocket subscribe without auth
        match request.method.as_str() {
            "GET" => Ok(vec![Signal::verdict(VerdictKind::Accept {
                identity: Identity::RelayRead,
            })]),
            "WS" if is_subscribe_only(&request) => Ok(vec![Signal::verdict(VerdictKind::Accept {
                identity: Identity::RelayRead,
            })]),
            _ => Ok(vec![Signal::verdict(VerdictKind::Reject {
                reason: "Write operations require authentication".into(),
                status: 401,
            })]),
        }
    }
}
```

### Agent-to-Agent Auth for Paid Feeds

Paid feed subscriptions use the same agent token mechanism. The subscribing agent's token is validated by the relay, and payment is recorded against the agent's budget.

---

## 7. AuthorizeCell: Workspace Membership and Role Verification

After a Verify Cell accepts a request, the `AuthorizeCell` checks that the authenticated identity is a member of this workspace and that their role has sufficient grants for the requested route. This is the final Cell in the auth pipeline before the route handler.

### 7.1 AuthorizeCell Implementation

```rust
pub struct AuthorizeCell {
    /// Path to the workspace user directory.
    users_dir: PathBuf,
}

/// A user record stored in .roko/users/{identifier}.json.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UserRecord {
    pub identifier: String,
    pub identifier_type: IdentifierType,
    pub role: WorkspaceRole,
    pub joined_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum IdentifierType {
    Email,
    Wallet,
}

impl AuthorizeCell {
    pub fn new(users_dir: PathBuf) -> Self {
        Self { users_dir }
    }

    /// Resolve a workspace role from an authenticated identity.
    fn resolve_role(&self, identity: &Identity) -> Result<Option<UserRecord>, CellError> {
        match identity {
            Identity::Privy { email, wallet, .. } => {
                // Try email first, then wallet
                if let Some(email) = email {
                    if let Some(record) = self.load_user(email)? {
                        return Ok(Some(record));
                    }
                }
                if let Some(wallet) = wallet {
                    if let Some(record) = self.load_user(wallet)? {
                        return Ok(Some(record));
                    }
                }
                // Check pending invitations
                if let Some(email) = email {
                    if let Some(invited_role) = self.check_invitation(email)? {
                        let record = self.create_user(email, IdentifierType::Email, invited_role)?;
                        return Ok(Some(record));
                    }
                }
                Ok(None)
            }
            Identity::ApiKey { owner, scopes } => {
                // API keys carry their own scopes; authorization is scope-based,
                // not role-based. The VerifyApiKey Cell already checked scopes.
                // Emit Accept with a synthetic Admin/Member role based on scopes.
                let role = if scopes.contains(&ApiKeyScope::Admin) {
                    WorkspaceRole::Owner
                } else {
                    WorkspaceRole::Member
                };
                Ok(Some(UserRecord {
                    identifier: owner.clone(),
                    identifier_type: IdentifierType::Email,
                    role,
                    joined_at: Utc::now(),
                }))
            }
            Identity::Agent { agent_id } => {
                // Agents are workspace-scoped by construction.
                // They get Member-level grants for agent-related routes.
                Ok(Some(UserRecord {
                    identifier: agent_id.to_string(),
                    identifier_type: IdentifierType::Email,
                    role: WorkspaceRole::Member,
                    joined_at: Utc::now(),
                }))
            }
            Identity::RelayRead => {
                // Relay read-public: Viewer-level grants (read-only routes).
                Ok(Some(UserRecord {
                    identifier: "relay-read".into(),
                    identifier_type: IdentifierType::Email,
                    role: WorkspaceRole::Viewer,
                    joined_at: Utc::now(),
                }))
            }
        }
    }

    /// Load a user record from .roko/users/{identifier}.json.
    fn load_user(&self, identifier: &str) -> Result<Option<UserRecord>, CellError> {
        let path = self.users_dir.join(format!("{}.json", identifier));
        if path.exists() {
            let data = std::fs::read_to_string(&path)
                .map_err(|e| CellError::Internal(format!("read user: {e}")))?;
            let record: UserRecord = serde_json::from_str(&data)
                .map_err(|e| CellError::Internal(format!("parse user: {e}")))?;
            Ok(Some(record))
        } else {
            Ok(None)
        }
    }

    /// Check pending invitations in .roko/users/invitations.json.
    fn check_invitation(&self, identifier: &str) -> Result<Option<WorkspaceRole>, CellError> {
        let path = self.users_dir.join("invitations.json");
        if !path.exists() { return Ok(None); }
        let data = std::fs::read_to_string(&path)
            .map_err(|e| CellError::Internal(format!("read invitations: {e}")))?;
        let invitations: Vec<Invitation> = serde_json::from_str(&data)
            .map_err(|e| CellError::Internal(format!("parse invitations: {e}")))?;
        Ok(invitations.iter()
            .find(|inv| inv.identifier == identifier)
            .map(|inv| inv.role.clone()))
    }

    /// Create a new user record (e.g., from accepted invitation).
    fn create_user(
        &self,
        identifier: &str,
        id_type: IdentifierType,
        role: WorkspaceRole,
    ) -> Result<UserRecord, CellError> {
        let record = UserRecord {
            identifier: identifier.to_string(),
            identifier_type: id_type,
            role,
            joined_at: Utc::now(),
        };
        let path = self.users_dir.join(format!("{}.json", identifier));
        let data = serde_json::to_string_pretty(&record)
            .map_err(|e| CellError::Internal(format!("serialize user: {e}")))?;
        std::fs::write(&path, data)
            .map_err(|e| CellError::Internal(format!("write user: {e}")))?;
        Ok(record)
    }

    /// Check if a role has grants for the requested route.
    fn check_grants(
        &self,
        role: &WorkspaceRole,
        method: &str,
        path: &str,
    ) -> bool {
        let grants = role_to_grants(role);
        grants.iter().any(|grant| {
            let method_ok = grant.methods.contains(&"*".to_string())
                || grant.methods.iter().any(|m| m == method);
            let path_ok = grant.path_pattern == "*"
                || path_matches(&grant.path_pattern, path);
            method_ok && path_ok
        })
    }
}

/// Map workspace roles to route grants.
fn role_to_grants(role: &WorkspaceRole) -> Vec<RouteGrant> {
    match role {
        WorkspaceRole::Owner => vec![
            RouteGrant { methods: vec!["*".into()], path_pattern: "*".into() },
        ],
        WorkspaceRole::Admin => vec![
            RouteGrant { methods: vec!["*".into()], path_pattern: "/api/agents/*".into() },
            RouteGrant { methods: vec!["*".into()], path_pattern: "/api/plans/*".into() },
            RouteGrant { methods: vec!["*".into()], path_pattern: "/api/prd/*".into() },
            RouteGrant { methods: vec!["GET".into()], path_pattern: "/api/secrets/*".into() },
            RouteGrant { methods: vec!["GET".into(), "POST".into()], path_pattern: "/api/team/*".into() },
            RouteGrant { methods: vec!["GET".into()], path_pattern: "/api/config/*".into() },
            RouteGrant { methods: vec!["GET".into()], path_pattern: "*".into() },
        ],
        WorkspaceRole::Member => vec![
            RouteGrant { methods: vec!["*".into()], path_pattern: "/api/agents/*".into() },
            RouteGrant { methods: vec!["*".into()], path_pattern: "/api/plans/*".into() },
            RouteGrant { methods: vec!["*".into()], path_pattern: "/api/prd/*".into() },
            RouteGrant { methods: vec!["GET".into()], path_pattern: "*".into() },
        ],
        WorkspaceRole::Viewer => vec![
            RouteGrant { methods: vec!["GET".into()], path_pattern: "*".into() },
        ],
    }
}

impl Cell for AuthorizeCell {
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Verify] }
    fn name(&self) -> &str { "authorize" }

    fn input_schema(&self) -> TypeSchema {
        TypeSchema::named("Verdict<Identity>")  // accepted identity from auth stage
    }
    fn output_schema(&self) -> TypeSchema {
        TypeSchema::named("Verdict<AuthorizedIdentity>")
    }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let verdict = extract_verdict(&input)?;
        let identity = match &verdict {
            VerdictKind::Accept { identity } => identity,
            // Should not reach AuthorizeCell without an Accept, but fail closed.
            _ => return Ok(vec![Signal::verdict(VerdictKind::Reject {
                reason: "No authenticated identity".into(),
                status: 401,
            })]),
        };

        let request = extract_request(&input)?;

        // First-run bootstrap: if no users exist, first login becomes owner.
        if self.users_dir.exists() && is_empty_dir(&self.users_dir) {
            if let Identity::Privy { email: Some(email), .. } = identity {
                self.create_user(email, IdentifierType::Email, WorkspaceRole::Owner)?;
            }
        }

        // Resolve role from identity
        match self.resolve_role(identity)? {
            Some(record) => {
                // Check grants against the requested route
                if self.check_grants(&record.role, &request.method, &request.path) {
                    Ok(vec![Signal::verdict(VerdictKind::Accept {
                        identity: Identity::Authorized(AuthorizedIdentity {
                            identity: identity.clone(),
                            role: record.role,
                            grants: role_to_grants(&record.role),
                        }),
                    })])
                } else {
                    Ok(vec![Signal::verdict(VerdictKind::Reject {
                        reason: format!(
                            "Role {:?} does not have permission for {} {}",
                            record.role, request.method, request.path
                        ),
                        status: 403,
                    })])
                }
            }
            None => Ok(vec![Signal::verdict(VerdictKind::Reject {
                reason: "Not a member of this workspace".into(),
                status: 403,
            })]),
        }
    }
}
```

---

## 8. Device Flow for Headless Machines

For CI servers, remote VMs, and other environments without a browser:

```bash
roko login https://my-roko.up.railway.app
# -> Visit https://my-roko.up.railway.app/auth/device
#    Enter code: ABCD-EFGH
# -> Polls until approved
# -> Token stored in OS keychain
```

### 8.1 Device Flow Protocol

```
CLI (headless machine)              roko-serve              User's browser
        |                               |                        |
        | POST /auth/device/begin       |                        |
        | ----------------------------> |                        |
        |                               |                        |
        | { device_code: "ABCD-EFGH",   |                        |
        |   poll_url: "/auth/device/    |                        |
        |            poll?code=...",    |                        |
        |   expires_in: 600 }           |                        |
        | <---------------------------- |                        |
        |                               |                        |
        | Print: "Visit .../auth/device |                        |
        |   Enter code: ABCD-EFGH"      |                        |
        |                               |                        |
        |                               |  User visits URL       |
        |                               | <--------------------- |
        |                               |  Privy login modal     |
        |                               | --------------------> |
        |                               |  JWT from Privy        |
        |                               | <--------------------- |
        |                               |  Enter device code     |
        |                               | <--------------------- |
        |                               |                        |
        | GET /auth/device/poll?code=.. |                        |
        | ----------------------------> |                        |
        |                               | (pending...)           |
        | { status: "pending" }         |                        |
        | <---------------------------- |                        |
        |                               |                        |
        | (poll again after 5s)         | (user approves)        |
        | GET /auth/device/poll?code=.. |                        |
        | ----------------------------> |                        |
        |                               |                        |
        | { status: "approved",         |                        |
        |   token: "sk_roko_..." }      |                        |
        | <---------------------------- |                        |
        |                               |                        |
        | Store token in OS keychain    |                        |
```

- Device codes expire after 10 minutes.
- Polling interval: 5 seconds.
- The token issued through device flow has the same scopes as the approving user's role.

---

## 9. Team Workspace Sharing

A deployed roko instance is private by default. The owner can invite teammates by email or wallet address.

### 9.1 Authorization Flow

```
User opens dashboard
  -> Privy login modal (email/Google/Apple/wallet) -- open, anyone can log in
  -> Privy issues JWT (contains privy_user_id + email)
  -> Dashboard sends JWT to THIS roko instance
  -> VerifyJwt Cell validates JWT signature (public JWKS, no secret)
  -> AuthorizeCell extracts email from identity
  -> Looks up email in .roko/users/
     -> Found? -> authorize with stored role
     -> Email matches pending invitation? -> auto-create user with invited role
     -> Not found, no invitation? -> 403 "Not a member of this workspace"
```

### 9.2 Invitation Flow

1. Owner goes to Settings > Team, types `sarah@example.com`, picks role "Member"
2. Dashboard calls `POST /api/team/invite` on the roko instance
3. Roko-serve stores invitation in `.roko/users/invitations.json` (local to this instance)
4. Dashboard shows a shareable link: `https://your-roko.up.railway.app`
5. Owner sends the link to Sarah (email, Slack, whatever)
6. Sarah opens link -> Privy login -> logs in with `sarah@example.com`
7. Privy issues JWT -> dashboard sends to roko-serve
8. AuthorizeCell sees email matches invitation -> creates user record with "member" role
9. Sarah sees the dashboard with member-level permissions

### 9.3 Four Workspace Roles

| Role | Agents | Plans | Secrets | Team | System |
|---|---|---|---|---|---|
| `owner` | full | full | full | manage | full |
| `admin` | full | full | view | invite | view |
| `member` | full | full | -- | -- | view |
| `viewer` | view | view | -- | -- | view |

Roles are stored locally in `.roko/users/{email}.json`. No Privy custom_metadata needed.

### 9.4 First-Run Bootstrap

1. User clicks "Deploy on Railway" -> roko deploys with `PRIVY_APP_ID` already set
2. User visits dashboard URL -> Privy login modal
3. User logs in (email/Google/Apple)
4. AuthorizeCell sees no users in `.roko/users/` -> first user becomes Owner automatically
5. Owner configures provider keys (Settings > Provider Keys)
6. Owner invites team members (Settings > Team)

### 9.5 Revoking Access

1. Owner removes Sarah from team (`DELETE /api/team/members/:id`)
2. Roko-serve deletes Sarah's user record from `.roko/users/`
3. Sarah's existing Privy JWT still works (Privy doesn't know about roko-serve's user table)
4. On Sarah's next API call: AuthorizeCell looks up her email -> not found -> 403 immediately
5. Dashboard shows "You are no longer a member of this workspace"

Revocation is instant from roko-serve's perspective. Sarah can still authenticate with Privy (it's open login), but AuthorizeCell won't let her in.

### 9.6 Wallet-Based Invitations

Invitations can be by wallet address in addition to email:

```json
{ "identifier": "0x7f3b...2c4a", "type": "wallet", "role": "member" }
```

AuthorizeCell matches the wallet address from the Privy JWT's linked accounts.

### 9.7 API Routes

```
POST /api/team/invite        -- invite by email or wallet (owner/admin only)
  Body: { "identifier": "alice@example.com", "type": "email", "role": "member" }
  -> Stores invitation locally in .roko/users/invitations.json
  -> Returns: { "invited": true }

GET /api/team/members         -- list team members (any authenticated user)
  -> [{ "email": "will@...", "role": "owner", "joined_at": "..." },
     { "email": "alice@...", "role": "member", "joined_at": "..." }]

PUT /api/team/members/:id     -- change role (owner/admin only)
  Body: { "role": "admin" }

DELETE /api/team/members/:id  -- remove from team (owner/admin only)
  -> Deletes user record, next API call gets 403

GET /api/team/me              -- current user's role and permissions
```

### 9.8 Dashboard UX

```
Settings > Team

+-------------------------------------------------------------+
| Team Members                                                 |
|                                                              |
| will@nunchi.dev              owner    Joined 2d ago          |
| alice@example.com            member   Joined 1d ago          |
| bob@example.com              admin    Joined 3h ago          |
|                                                              |
| Pending Invitations                                          |
| carol@example.com            member   Invited 1h ago         |
|                                                              |
| +----------------------------------------------------------+|
| | Invite teammate                                           ||
| | Email or wallet: [                        ] Role: [member]||
| | [Send Invite]                                             ||
| +----------------------------------------------------------+|
+-------------------------------------------------------------+
```

---

## 10. Secret and API Key Management

### 10.1 Storage Hierarchy (3-Tier Priority)

> **Terminology note**: This 3-tier priority ordering (env > secrets.toml > config) applies specifically to **auth secrets** (provider API keys, integration tokens). It is distinct from the **config priority system** defined in [19-CONFIG](19-CONFIG.md), which uses a 4-tier ordering (CLI > env > TOML > evolved) for general configuration values. The two systems are independent: auth secrets resolve through the hierarchy below, while config values (including non-secret fields in `roko.toml`) resolve through the config Compose Cell described in doc 19.

```
Priority    Source              Where
--------    ------              -----
1 (highest) Environment vars   ANTHROPIC_API_KEY, PERPLEXITY_API_KEY, etc.
2           Secrets store       .roko/secrets.toml (encrypted at rest)
3           Config file         roko.toml [providers] section (not recommended)
```

### 10.2 Secrets Store Format

```toml
# .roko/secrets.toml
# Encrypted with age (https://age-encryption.org)
# Key derived from machine identity or user passphrase

[llm]
anthropic = "sk-ant-..."
perplexity = "pplx-..."
gemini = "AIza..."
openrouter = "sk-or-..."
moonshot = "sk-..."
zai = "..."

[integration]
github = "ghp_..."
slack = "xoxb-..."

[infra]
fly_api_token = "fo1_..."
railway_token = "..."
```

### 10.3 CLI Secret Management

```bash
# Set a secret (reads from stdin, never in shell history)
echo "sk-ant-xyz" | roko config secrets set llm.anthropic

# Interactive prompt
roko config secrets set llm.anthropic
# Enter secret: ****

# List configured secrets (keys, never values)
roko config secrets list
# NAMESPACE    KEY          SOURCE        STATUS
# llm          anthropic    secrets.toml  * valid
# llm          perplexity   env var       * valid
# integration  github       secrets.toml  * valid
# llm          gemini       --            o not set

# Validate all secrets
roko config check-secrets
# Anthropic: valid (claude-sonnet-4-6 accessible)
# Perplexity: valid
# GitHub: valid (repo scope, expires 2026-06-01)
# Gemini: not configured
```

### 10.4 Dashboard Secret Management

Settings > Provider Keys page. Each provider shows a status indicator (connected / not set / invalid). Users paste keys into a form. The dashboard sends them to `POST /api/secrets/:ns/:key`.

Test button calls `POST /api/secrets/:ns/:key/test` -- the server makes a minimal API call to the provider and returns connection status.

**Client-only mode**: keys stored in `localStorage`, sent via `X-Provider-Keys` header per request. The server uses them but never persists them.

---

## 11. The Complete Auth Pipeline as a Graph

```toml
[graph]
id = "auth-pipeline"
description = "Request authentication (4 Verify Cells) + authorization (1 Verify Cell)"

# --- Stage 1: Authentication (short-circuits on first accept) ---

[[graph.cells]]
id = "verify-jwt"
protocol = "Verify"
input_schema = "AuthRequest"
output_schema = "Verdict<Identity::Privy>"
description = "Privy JWT validation with JWKS caching"

[[graph.cells]]
id = "verify-api-key"
protocol = "Verify"
input_schema = "AuthRequest"
output_schema = "Verdict<Identity::ApiKey>"
description = "API key validation with scope checking"

[[graph.cells]]
id = "verify-agent-token"
protocol = "Verify"
input_schema = "AuthRequest"
output_schema = "Verdict<Identity::Agent>"
description = "Agent bearer token validation with grace period"

[[graph.cells]]
id = "verify-relay-read"
protocol = "Verify"
input_schema = "AuthRequest"
output_schema = "Verdict<Identity::RelayRead>"
description = "Relay read-public passthrough for GET operations"

# --- Stage 2: Authorization ---

[[graph.cells]]
id = "authorize"
protocol = "Verify"
input_schema = "Verdict<Identity>"
output_schema = "Verdict<AuthorizedIdentity>"
description = "Post-auth: check workspace membership, resolve role, verify route grants"
config = { users_dir = ".roko/users/" }

# --- Edges: Authentication chain (skip = pass to next) ---

[[graph.edges]]
from = "verify-jwt.verdict"
to = "verify-api-key.in"
condition = "skip"       # only proceed to next if JWT cell skipped

[[graph.edges]]
from = "verify-api-key.verdict"
to = "verify-agent-token.in"
condition = "skip"

[[graph.edges]]
from = "verify-agent-token.verdict"
to = "verify-relay-read.in"
condition = "skip"

# --- Edges: All accepted identities flow to authorization ---

[[graph.edges]]
from = "verify-jwt.verdict"
to = "authorize.in"
condition = "accept"

[[graph.edges]]
from = "verify-api-key.verdict"
to = "authorize.in"
condition = "accept"

[[graph.edges]]
from = "verify-agent-token.verdict"
to = "authorize.in"
condition = "accept"

[[graph.edges]]
from = "verify-relay-read.verdict"
to = "authorize.in"
condition = "accept"
```

---

## 12. Acceptance Criteria

| # | Criterion | Verification |
|---|---|---|
| A-1 | Privy JWT validated against JWKS endpoint | Integration test with mock JWKS |
| A-2 | JWKS cache refreshes after 1 hour | Unit test with mocked clock |
| A-3 | Key rotation: JWT fails cached keys, succeeds after single JWKS refetch | Integration test |
| A-4 | Stale JWKS used when endpoint is down (stale-while-revalidate) | Integration test |
| A-5 | API key with `read` scope can GET any route | Integration test |
| A-6 | API key with `read` scope gets 403 on POST to agent routes | Integration test |
| A-7 | API key with `admin` scope can access all routes | Integration test |
| A-8 | Insufficient scope response includes required/has/route fields | Unit test |
| A-9 | Agent token SHA-256 hashed before storage | Unit test: plaintext not in DB |
| A-10 | Agent token plaintext returned exactly once at issuance | Integration test |
| A-11 | Agent token rejected after expiry (30 days default) | Unit test with mocked clock |
| A-12 | Token rotation: old + new both valid during 5-minute grace period | Integration test |
| A-13 | Token rotation: old token rejected after grace period expires | Integration test |
| A-14 | Grace period returns the **real agent_id**, not a placeholder string | Unit test: verify `Identity::Agent { agent_id }` matches original agent |
| A-15 | Relay read operations succeed without auth | Integration test |
| A-16 | Relay write operations require agent token | Integration test |
| A-17 | Device flow: code expires after 10 minutes | Unit test |
| A-18 | Device flow: poll returns token after browser approval | Integration test |
| A-19 | First user becomes owner automatically | Integration test: empty `.roko/users/` |
| A-20 | Invitation flow: matching email auto-creates user with invited role | Integration test |
| A-21 | Non-member gets 403 even with valid Privy JWT | Integration test |
| A-22 | Revoked member gets 403 immediately | Integration test |
| A-23 | Wallet-based invitation matches JWT linked accounts | Integration test |
| A-24 | Secrets priority: env var > secrets.toml > config file | Unit test |
| A-25 | `roko config check-secrets` validates each provider | Integration test |
| A-26 | Auth pipeline short-circuits on first acceptance | Unit test |
| A-27 | Auth pipeline returns 401 when all Verify Cells skip | Unit test |
| A-28 | AuthorizeCell rejects non-member with 403 (not 401) | Integration test |
| A-29 | AuthorizeCell resolves correct role from `.roko/users/{email}.json` | Unit test |
| A-30 | AuthorizeCell auto-creates user on invitation match | Integration test |
| A-31 | AuthorizeCell emits `AuthorizedIdentity` with role and grants on accept | Unit test: verify output Signal payload |
| A-32 | Owner role grants access to all routes | Unit test: `role_to_grants(Owner)` covers `*` |
| A-33 | Viewer role grants only GET access | Unit test: `role_to_grants(Viewer)` rejects POST/PUT/DELETE |
| A-34 | All 5 Verify Cells declare `input_schema` and `output_schema` (typed I/O) | Compile-time check or unit test |
| A-35 | TOML Graph definition for auth-pipeline loads and validates edge conditions | Unit test: parse + validate |

---

## 13. Cross-References

| Topic | Document | Section |
|---|---|---|
| Verify protocol (Cell) | [doc-02](02-CELL.md) | SS3.3 |
| Pipeline Graph wiring | [doc-03](03-GRAPH.md) | SS2 |
| Capability intersection | [doc-16](16-SECURITY.md) | SS2-3 |
| Fail-closed principle | [doc-16](16-SECURITY.md) | SS1 |
| HTTP control plane routes | roko-serve | `crates/roko-serve/src/routes/` |
| Relay protocol | [doc-11](11-CONNECTIVITY.md) | SS3 |
| Payment auth for feeds | [doc-18](18-PAYMENTS.md) | SS1-2 |
| Config priority system (separate from secret priority) | [doc-19](19-CONFIG.md) | SS1 |
