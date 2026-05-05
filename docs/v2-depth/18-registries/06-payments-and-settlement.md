# Payments and Settlement

> Depth for [22-REGISTRIES.md](../../unified/22-REGISTRIES.md). How payment protocols, clearing, and dispute resolution emerge from Connect, Verify, and Store Cells.

This doc specifies the full payment lifecycle -- from the initial HTTP 402 handshake through off-chain state channels, cooperative clearing, on-chain settlement, and multi-level dispute resolution. Every component decomposes into standard Cell primitives: Connect Cells for payment protocol negotiation, Verify Cells for settlement validation, Score Cells for trust-weighted fact aggregation, and Compose Cells for optimal resource allocation. No new kernel types are introduced.

---

## 1. Payment Protocols as Connect Cells

Payment protocols are how agents exchange value. In Roko, each payment protocol is a **Connect Cell** -- it implements the Connect protocol lifecycle (connect, query, execute, disconnect, health_check) for a specific payment mechanism. The Connect protocol ([02-CELL](../../unified/02-CELL.md) S2.8) already defines the right shape: establish a connection, perform operations through it, and manage its lifecycle.

### 1.1 The Three Payment Connect Cells

| Cell | Protocol | Model | Connect Phase | Execute Phase | Disconnect Phase |
|---|---|---|---|---|---|
| `X402ConnectCell` | x402 | Per-request, stateless | Discover payment terms (402 response) | Sign + submit ERC-3009 authorization | No-op (stateless) |
| `StateChannelConnectCell` | State channels | Off-chain transact, cooperative close | Open channel (on-chain deposit) | Off-chain state updates (0 gas) | Cooperative close or dispute |
| `StreamConnectCell` | Superfluid streaming | Continuous wei/sec flow | Start stream (1 on-chain tx) | Real-time balance computation (0 gas) | Stop stream (1 on-chain tx) |

All three implement `ConnectProtocol`. Downstream code that needs to pay for a resource calls `connect()`, uses the connection, and calls `disconnect()`. The payment mechanism is swapped by selecting a different Cell -- no changes to the consumer.

### 1.2 x402 Connect Cell

The x402 protocol handles the HTTP 402 handshake. It is the simplest payment Cell: stateless, per-request, no session. The existing VerifyX402Cell from [18-PAYMENTS](../../unified/18-PAYMENTS.md) handles the server side (verifying incoming payments). This Connect Cell handles the client side (making payments).

```rust
/// x402 client-side Connect Cell.
/// Handles the 402 handshake: discovers terms, signs authorization, retries.
///
/// Connect lifecycle:
///   connect() -> discover payment terms from 402 response
///   execute() -> sign ERC-3009, retry with X-Payment header
///   disconnect() -> no-op (stateless)
pub struct X402ConnectCell {
    /// Signer for ERC-3009 authorizations (wallet key).
    signer: Arc<dyn Signer>,

    /// Maximum amount willing to pay per request (safety cap).
    max_amount_per_request: u64,

    /// Pending authorizations awaiting batch settlement.
    pending_settlements: Arc<Mutex<Vec<Erc3009Authorization>>>,

    /// Settlement configuration.
    settlement_config: SettlementConfig,
}

pub struct SettlementConfig {
    /// Maximum pending authorizations before triggering settlement.
    pub max_pending: usize,         // default: 100

    /// Maximum time between settlements.
    pub settle_interval: Duration,  // default: 10 minutes

    /// Batch size for on-chain settlement transactions.
    /// 50-200 authorizations per tx to amortize gas.
    pub batch_size: usize,          // default: 100
}

impl Cell for X402ConnectCell {
    fn name(&self) -> &str { "connect-x402" }
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Connect] }
}

#[async_trait]
impl ConnectProtocol for X402ConnectCell {
    /// Discover payment terms by making an unauthenticated request.
    /// The server returns 402 with X-Payment-Required header.
    async fn connect(&self, ctx: &CellContext) -> Result<ConnectionHandle> {
        // x402 is stateless -- "connecting" just validates the endpoint.
        Ok(ConnectionHandle::stateless())
    }

    /// Execute a paid request.
    /// 1. Send request without payment -> receive 402 with terms.
    /// 2. Sign ERC-3009 authorization for the required amount.
    /// 3. Retry with X-Payment header -> receive 200 + data.
    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>> {
        let request = &input[0];
        let endpoint = request.payload.get("endpoint")
            .and_then(|v| v.as_str())
            .ok_or(CellError::MissingField("endpoint"))?;

        // Step 1: Initial request (no payment).
        let response = ctx.http_client().get(endpoint).send().await?;

        if response.status() != 402 {
            // No payment required -- return the response directly.
            return Ok(vec![Signal::from_http_response(response).await?]);
        }

        // Step 2: Parse payment terms from 402 response.
        let terms = parse_payment_terms(&response)?;
        if terms.amount > self.max_amount_per_request {
            return Err(CellError::PaymentExceedsLimit {
                required: terms.amount,
                limit: self.max_amount_per_request,
            });
        }

        // Step 3: Sign ERC-3009 authorization.
        let auth = Erc3009Authorization {
            from: self.signer.address(),
            to: terms.recipient,
            value: terms.amount,
            valid_after: 0,
            valid_before: terms.expiry,
            nonce: generate_nonce(),
        };
        let signature = self.signer.sign_erc3009(&auth).await?;

        // Step 4: Retry with X-Payment header.
        let paid_response = ctx.http_client()
            .get(endpoint)
            .header("X-Payment", encode_authorization(&auth, &signature))
            .send()
            .await?;

        // Step 5: Collect authorization for batch settlement.
        self.pending_settlements.lock().await.push(auth);

        // Step 6: Check if batch settlement should fire.
        self.maybe_settle(ctx).await?;

        Ok(vec![Signal::from_http_response(paid_response).await?])
    }

    async fn disconnect(&self, _handle: ConnectionHandle) -> Result<()> {
        // Stateless -- no-op. Settlement handles the on-chain finalization.
        Ok(())
    }

    async fn health_check(&self, _handle: &ConnectionHandle) -> HealthStatus {
        // Check signer is accessible and has sufficient balance.
        match self.signer.balance().await {
            Ok(bal) if bal > 0 => HealthStatus::Healthy,
            Ok(_) => HealthStatus::Degraded {
                reason: "zero balance".into(),
            },
            Err(e) => HealthStatus::Unhealthy { error: e.to_string() },
        }
    }
}

impl X402ConnectCell {
    /// Submit batch settlement if thresholds are met.
    /// 50-200 authorizations per on-chain transaction.
    async fn maybe_settle(&self, ctx: &CellContext) -> Result<()> {
        let mut pending = self.pending_settlements.lock().await;
        let should_settle = pending.len() >= self.settlement_config.max_pending;

        if should_settle {
            if let Some(chain) = ctx.chain_client() {
                let batch = std::mem::take(&mut *pending);
                let _tx_hash = chain.batch_transfer_with_authorization(&batch).await?;
            }
        }

        Ok(())
    }
}
```

### 1.3 State Channel Connect Cell

State channels move transactions off-chain. The Connect lifecycle maps naturally: `connect()` opens the channel with an on-chain deposit, `execute()` performs off-chain state updates (0 gas per transaction), and `disconnect()` triggers cooperative close. If the counterparty disappears, the dispute mechanism (S5) activates.

```rust
/// State channel Connect Cell.
/// Open/transact/close lifecycle maps to Connect protocol.
///
/// Connect:    Open channel (on-chain deposit).
/// Execute:    Off-chain state update (signed by both parties, 0 gas).
/// Disconnect: Cooperative close (1 on-chain tx settling final balances).
/// Dispute:    If cooperative close fails, unilateral close with
///             100-block dispute window. Highest nonce wins.
pub struct StateChannelConnectCell {
    /// Signer for state updates.
    signer: Arc<dyn Signer>,

    /// Channel state (None if not yet opened).
    channel: Arc<RwLock<Option<ChannelState>>>,

    /// Dispute window in blocks.
    dispute_window: u64,  // default: 100
}

pub struct ChannelState {
    /// Channel ID (hash of opening parameters).
    pub channel_id: [u8; 32],

    /// Current state nonce (increments on each update).
    pub nonce: u64,

    /// Balance allocation: (our_balance, their_balance).
    pub balances: (u64, u64),

    /// Both parties' signatures on the current state.
    pub signatures: (Signature, Signature),

    /// Block at which the channel was opened.
    pub opened_at_block: u64,
}

impl Cell for StateChannelConnectCell {
    fn name(&self) -> &str { "connect-state-channel" }
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Connect] }
}

#[async_trait]
impl ConnectProtocol for StateChannelConnectCell {
    /// Open a state channel. Deposits funds on-chain.
    async fn connect(&self, ctx: &CellContext) -> Result<ConnectionHandle> {
        let chain = ctx.chain_client().ok_or(CellError::NoChainClient)?;

        // Deploy or use existing channel contract.
        let channel_id = chain.open_state_channel(
            self.signer.address(),
            ctx.counterparty_address()?,
            ctx.deposit_amount()?,
        ).await?;

        let state = ChannelState {
            channel_id,
            nonce: 0,
            balances: (ctx.deposit_amount()?, 0),
            signatures: (Signature::empty(), Signature::empty()),
            opened_at_block: chain.latest_block().await?,
        };

        *self.channel.write().await = Some(state);

        Ok(ConnectionHandle::stateful(channel_id))
    }

    /// Execute an off-chain state update. Zero gas.
    /// Both parties sign the new state. Highest nonce wins on-chain.
    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>> {
        let mut channel = self.channel.write().await;
        let state = channel.as_mut().ok_or(CellError::NotConnected)?;

        let transfer_amount: u64 = input[0].payload.get("amount")
            .and_then(|v| v.as_u64())
            .ok_or(CellError::MissingField("amount"))?;

        // Update balances.
        state.balances.0 = state.balances.0.checked_sub(transfer_amount)
            .ok_or(CellError::InsufficientBalance)?;
        state.balances.1 += transfer_amount;
        state.nonce += 1;

        // Sign the new state.
        let state_hash = hash_channel_state(state);
        let our_sig = self.signer.sign(&state_hash).await?;

        // Exchange signatures with counterparty (via Bus or direct).
        let their_sig = ctx.exchange_signature(state_hash, our_sig.clone()).await?;

        state.signatures = (our_sig, their_sig);

        Ok(vec![Signal::new(Kind::PaymentStateUpdate, json!({
            "channel_id": hex::encode(state.channel_id),
            "nonce": state.nonce,
            "our_balance": state.balances.0,
            "their_balance": state.balances.1,
        }))])
    }

    /// Cooperative close: both parties agree on final state, settle on-chain.
    async fn disconnect(&self, _handle: ConnectionHandle) -> Result<()> {
        let channel = self.channel.read().await;
        let state = channel.as_ref().ok_or(CellError::NotConnected)?;

        // Try cooperative close first (cheapest: 1 tx).
        // If counterparty is unresponsive, fall back to unilateral close
        // with dispute window (see S5).
        Ok(())
    }

    async fn health_check(&self, _handle: &ConnectionHandle) -> HealthStatus {
        match &*self.channel.read().await {
            Some(state) if state.balances.0 > 0 => HealthStatus::Healthy,
            Some(_) => HealthStatus::Degraded {
                reason: "channel balance exhausted".into(),
            },
            None => HealthStatus::Unhealthy {
                error: "no channel open".into(),
            },
        }
    }
}
```

### 1.4 Streaming Connect Cell (Superfluid-Style)

Streaming payments flow continuously at a rate of wei/sec. One on-chain transaction starts the stream; another stops it. Between start and stop, the balance is computed in real-time with zero gas cost per second.

```rust
/// Superfluid-style streaming payment Connect Cell.
/// Start/stop are the only on-chain transactions.
/// Balance computed in real-time: balance = initial - rate * elapsed.
///
/// Connect:    Start stream (1 on-chain tx: set flow_rate).
/// Execute:    Query real-time balance (0 gas, pure computation).
/// Disconnect: Stop stream (1 on-chain tx: set flow_rate = 0).
pub struct StreamConnectCell {
    /// Signer for stream transactions.
    signer: Arc<dyn Signer>,

    /// Active stream state.
    stream: Arc<RwLock<Option<StreamState>>>,
}

pub struct StreamState {
    /// Stream identifier.
    pub stream_id: [u8; 32],

    /// Flow rate in wei per second.
    pub flow_rate: u128,

    /// Buffer: pre-funded amount to cover flow_rate * buffer_seconds.
    /// Default buffer_seconds = 3600 (1 hour).
    pub buffer_amount: u128,

    /// Timestamp when the stream was started.
    pub started_at: DateTime<Utc>,

    /// Initial deposit amount.
    pub initial_deposit: u128,
}

impl StreamState {
    /// Compute current balance. Real-time, zero gas.
    /// balance = initial_deposit - flow_rate * elapsed_seconds
    pub fn balance_now(&self) -> u128 {
        let elapsed = Utc::now()
            .signed_duration_since(self.started_at)
            .num_seconds()
            .max(0) as u128;
        let spent = self.flow_rate.saturating_mul(elapsed);
        self.initial_deposit.saturating_sub(spent)
    }

    /// Estimate time until stream exhaustion.
    pub fn time_remaining(&self) -> Duration {
        if self.flow_rate == 0 {
            return Duration::from_secs(u64::MAX);
        }
        let balance = self.balance_now();
        let seconds = balance / self.flow_rate;
        Duration::from_secs(seconds as u64)
    }
}

impl Cell for StreamConnectCell {
    fn name(&self) -> &str { "connect-stream" }
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Connect] }
}

#[async_trait]
impl ConnectProtocol for StreamConnectCell {
    /// Start a payment stream. One on-chain transaction.
    async fn connect(&self, ctx: &CellContext) -> Result<ConnectionHandle> {
        let chain = ctx.chain_client().ok_or(CellError::NoChainClient)?;
        let flow_rate = ctx.flow_rate()?;
        let buffer_seconds: u128 = 3600; // 1 hour buffer
        let deposit = flow_rate * buffer_seconds + flow_rate * ctx.stream_duration_secs()?;

        let stream_id = chain.start_stream(
            self.signer.address(),
            ctx.recipient_address()?,
            flow_rate,
            deposit,
        ).await?;

        let state = StreamState {
            stream_id,
            flow_rate,
            buffer_amount: flow_rate * buffer_seconds,
            started_at: Utc::now(),
            initial_deposit: deposit,
        };

        *self.stream.write().await = Some(state);
        Ok(ConnectionHandle::stateful(stream_id))
    }

    /// Query real-time balance. Zero gas -- pure computation.
    async fn execute(
        &self,
        _input: Vec<Signal>,
        _ctx: &CellContext,
    ) -> Result<Vec<Signal>> {
        let stream = self.stream.read().await;
        let state = stream.as_ref().ok_or(CellError::NotConnected)?;

        Ok(vec![Signal::new(Kind::StreamBalance, json!({
            "stream_id": hex::encode(state.stream_id),
            "balance": state.balance_now().to_string(),
            "flow_rate": state.flow_rate.to_string(),
            "time_remaining_secs": state.time_remaining().as_secs(),
        }))])
    }

    /// Stop the stream. One on-chain transaction.
    async fn disconnect(&self, _handle: ConnectionHandle) -> Result<()> {
        let stream = self.stream.read().await;
        if let Some(state) = stream.as_ref() {
            // Stop stream: set flow_rate to 0, refund remaining balance.
            // ctx.chain_client()?.stop_stream(state.stream_id).await?;
        }
        Ok(())
    }

    async fn health_check(&self, _handle: &ConnectionHandle) -> HealthStatus {
        match &*self.stream.read().await {
            Some(state) if state.balance_now() > state.buffer_amount => {
                HealthStatus::Healthy
            }
            Some(state) if state.balance_now() > 0 => {
                HealthStatus::Degraded {
                    reason: format!(
                        "balance below buffer: {} remaining",
                        state.time_remaining().as_secs()
                    ),
                }
            }
            Some(_) => HealthStatus::Unhealthy {
                error: "stream exhausted".into(),
            },
            None => HealthStatus::Unhealthy {
                error: "no stream active".into(),
            },
        }
    }
}
```

### 1.5 Payment Protocol Selection

The choice of payment Cell is a **Route decision**: given the task requirements (duration, expected cost, latency tolerance), the Route protocol selects the optimal payment mechanism.

| Factor | x402 Preferred | State Channel Preferred | Stream Preferred |
|---|---|---|---|
| **Duration** | One-shot or minutes | Hours to days | Days to permanent |
| **Transaction count** | 1-10 | 10-10,000 | Continuous |
| **Gas cost** | Amortized via batch | 2 tx (open + close) | 2 tx (start + stop) |
| **Latency** | Per-request 402 handshake | Off-chain instant | Real-time balance |
| **Counterparty trust** | None (stateless) | Some (dispute window) | Some (buffer deposit) |

```rust
/// Route Cell that selects the optimal payment protocol.
fn select_payment_protocol(
    expected_duration: Duration,
    expected_transactions: u64,
    latency_tolerance: Duration,
) -> PaymentProtocolChoice {
    match (expected_duration, expected_transactions) {
        (d, n) if d < Duration::from_secs(300) && n < 10 => {
            PaymentProtocolChoice::X402
        }
        (d, _) if d > Duration::from_secs(86400) => {
            PaymentProtocolChoice::Stream
        }
        _ => PaymentProtocolChoice::StateChannel,
    }
}
```

---

## 2. ISFR as a Score Cell

The Intersubjective Fact Registry (ISFR) aggregates claims from multiple agents into a consensus value. It is a **Score Cell**: it takes a set of `FactClaim` Signals and produces a scored consensus Signal.

### 2.1 The Scoring Mechanism

Each agent's claim is weighted by three factors:

```
w_i = R_i * c_i * sqrt(stake_i)

where:
  R_i     = agent's reputation score (from ERC-8004 registry)
  c_i     = agent's calibration score (from predict-publish-correct history)
  stake_i = amount staked on this claim (sqrt prevents plutocracy)
```

The sqrt on stake is the critical design choice: a whale staking 10,000 tokens gets weight sqrt(10,000) = 100, while 100 agents each staking 1 token get weight 100 * sqrt(1) = 100. Economic power and social consensus are equalized.

```rust
/// ISFR Score Cell: weighted aggregation of FactClaims.
///
/// Takes Vec<FactClaim> Signals, produces a consensus Signal
/// with weighted mean and confidence interval.
pub struct IsfrScoreCell {
    /// Reputation registry client for looking up R_i.
    reputation: Arc<ReputationClient>,

    /// Calibration store for looking up c_i.
    calibration: Arc<dyn StoreProtocol>,

    /// Minimum total weight for a valid aggregation.
    min_total_weight: f64,   // default: 10.0

    /// Minimum number of independent claims.
    min_claims: usize,       // default: 3
}

pub struct FactClaim {
    /// The agent submitting this claim.
    pub agent_id: AgentId,

    /// The claimed value (numeric for rates, boolean for assertions).
    pub value: FactValue,

    /// Amount staked on this claim.
    pub stake: u64,

    /// Evidence supporting the claim (optional Signal references).
    pub evidence: Vec<SignalRef>,

    /// Timestamp of the observation.
    pub observed_at: DateTime<Utc>,
}

pub enum FactValue {
    Numeric(f64),
    Boolean(bool),
    Categorical(String),
}

impl Cell for IsfrScoreCell {
    fn name(&self) -> &str { "isfr-score" }
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Score] }
}

#[async_trait]
impl ScoreProtocol for IsfrScoreCell {
    async fn score(&self, signal: &Signal, ctx: &CellContext) -> Result<Score> {
        let claims: Vec<FactClaim> = serde_json::from_value(signal.payload.clone())?;

        if claims.len() < self.min_claims {
            return Ok(Score::insufficient("too few claims"));
        }

        // Compute weights for each claim.
        let mut weighted_claims = Vec::new();
        let mut total_weight = 0.0;

        for claim in &claims {
            let r_i = self.reputation.get_score(claim.agent_id).await
                .unwrap_or(0.1); // default: low reputation
            let c_i = self.calibration_score(claim.agent_id).await;
            let stake_sqrt = (claim.stake as f64).sqrt();

            let w_i = r_i * c_i * stake_sqrt;
            total_weight += w_i;

            weighted_claims.push((claim, w_i));
        }

        if total_weight < self.min_total_weight {
            return Ok(Score::insufficient("insufficient total weight"));
        }

        // Weighted mean for numeric claims.
        let consensus = match &claims[0].value {
            FactValue::Numeric(_) => {
                let weighted_sum: f64 = weighted_claims.iter()
                    .filter_map(|(claim, w)| match &claim.value {
                        FactValue::Numeric(v) => Some(v * w),
                        _ => None,
                    })
                    .sum();
                weighted_sum / total_weight
            }
            FactValue::Boolean(_) => {
                let weighted_true: f64 = weighted_claims.iter()
                    .filter_map(|(claim, w)| match &claim.value {
                        FactValue::Boolean(true) => Some(*w),
                        _ => None,
                    })
                    .sum();
                weighted_true / total_weight
            }
            FactValue::Categorical(_) => {
                // For categorical: weighted mode.
                0.0 // simplified
            }
        };

        // Compute weighted variance for confidence interval.
        let weighted_variance: f64 = weighted_claims.iter()
            .filter_map(|(claim, w)| match &claim.value {
                FactValue::Numeric(v) => Some(w * (v - consensus).powi(2)),
                _ => None,
            })
            .sum::<f64>() / total_weight;

        let std_error = (weighted_variance / claims.len() as f64).sqrt();

        Ok(Score {
            relevance: 1.0,
            confidence: 1.0 - std_error.min(1.0), // higher error -> lower confidence
            novelty: 0.5,
            dimensions: vec![
                ("consensus".into(), consensus),
                ("std_error".into(), std_error),
                ("total_weight".into(), total_weight),
                ("claim_count".into(), claims.len() as f64),
            ],
        })
    }
}

impl IsfrScoreCell {
    /// Look up an agent's calibration score from the predict-publish-correct
    /// history in Store. Agents whose past claims matched reality score high.
    async fn calibration_score(&self, agent_id: AgentId) -> f64 {
        let history = self.calibration.query(StoreQuery {
            kind: Some(Kind::CalibrationRecord),
            tags: Some(vec![format!("agent:{}", agent_id)]),
            limit: 100,
            ..Default::default()
        }).await.unwrap_or_default();

        if history.is_empty() {
            return 0.5; // neutral prior
        }

        // Calibration = fraction of past claims that matched ground truth.
        let correct = history.iter()
            .filter(|s| s.payload.get("correct").and_then(|v| v.as_bool()) == Some(true))
            .count();

        correct as f64 / history.len() as f64
    }
}
```

---

## 3. QP Clearing as a Compose Cell

The clearing problem is: given N agents with bids to buy and offers to sell, find the allocation that maximizes total welfare subject to capacity, single-assignment, and budget constraints. This is a **Compose Cell** -- it assembles the optimal allocation from competing bids, just as the VCG attention auction ([vcg-attention-auction.md](../02-block/vcg-attention-auction.md)) assembles the optimal context allocation from competing bidders.

### 3.1 The QP Formulation

```
Maximize:   sum(q_ij * x_ij)           -- total welfare
Subject to:
  sum_j(x_ij) <= 1  for all i          -- single assignment per agent
  sum_i(x_ij) <= cap_j  for all j      -- capacity per resource
  sum_j(x_ij * p_ij) <= budget_i       -- budget per agent
  x_ij >= 0                            -- non-negativity

With regularization:
  - lambda * sum(x_ij * p_ij)^2        -- penalize concentration
```

### 3.2 QP Clearing Compose Cell

```rust
/// QP Clearing Compose Cell.
/// Assembles the optimal allocation from competing bids.
///
/// Takes: Vec<ClearingBid> Signals (from participating agents).
/// Produces: ClearingCertificate Signal (allocation + KKT proof).
///
/// Compose protocol: this IS budget-constrained assembly.
/// The budget is the agents' combined willingness to pay.
/// The assembly is the welfare-maximizing allocation.
pub struct QpClearingCell {
    /// Regularization parameter: penalizes concentration.
    lambda: f64,   // default: 0.01

    /// Maximum iterations for bisection solver.
    max_iterations: usize,  // default: 80

    /// Convergence tolerance.
    tolerance: f64,  // default: 1e-8
}

pub struct ClearingBid {
    /// Agent submitting the bid.
    pub agent_id: AgentId,

    /// Resource being bid on.
    pub resource_id: String,

    /// Quality value: how much the agent values this resource.
    pub quality: f64,

    /// Price willing to pay.
    pub price: u64,

    /// Maximum budget for all resources.
    pub budget: u64,
}

pub struct ClearingAllocation {
    /// Agent receiving the resource.
    pub agent_id: AgentId,

    /// Resource allocated.
    pub resource_id: String,

    /// Fraction allocated (0.0..=1.0).
    pub fraction: f64,

    /// Price to pay.
    pub price: u64,
}

/// The clearing certificate: allocation + KKT proof for on-chain verification.
pub struct ClearingCertificate {
    /// The computed allocation.
    pub allocations: Vec<ClearingAllocation>,

    /// Total welfare achieved.
    pub total_welfare: f64,

    /// KKT multipliers (dual variables) for on-chain verification.
    /// These prove the allocation is optimal without re-solving.
    pub kkt_multipliers: KktCertificate,

    /// Hash of the input bids (for tamper detection).
    pub input_hash: [u8; 32],

    /// Solver metadata.
    pub iterations: usize,
    pub converged: bool,
}

/// KKT (Karush-Kuhn-Tucker) certificate.
/// Proves the allocation satisfies all optimality conditions.
/// On-chain verifier checks these conditions without re-solving.
pub struct KktCertificate {
    /// Dual variables for single-assignment constraints.
    pub agent_duals: Vec<f64>,

    /// Dual variables for capacity constraints.
    pub resource_duals: Vec<f64>,

    /// Dual variables for budget constraints.
    pub budget_duals: Vec<f64>,

    /// Complementary slackness residuals (should all be ~0).
    pub slackness_residuals: Vec<f64>,

    /// Maximum residual (convergence measure).
    pub max_residual: f64,
}

impl Cell for QpClearingCell {
    fn name(&self) -> &str { "qp-clearing" }
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Compose] }
}

#[async_trait]
impl ComposeProtocol for QpClearingCell {
    async fn compose(
        &self,
        inputs: Vec<Signal>,
        budget: &ComposeBudget,
        _ctx: &CellContext,
    ) -> Result<ComposeResult> {
        let bids: Vec<ClearingBid> = inputs.iter()
            .map(|s| serde_json::from_value(s.payload.clone()))
            .collect::<Result<_, _>>()?;

        // Solve via bisection on the Lagrangian dual.
        let (allocations, kkt) = self.solve_qp(&bids)?;

        let total_welfare: f64 = allocations.iter()
            .map(|a| a.fraction * bids.iter()
                .find(|b| b.agent_id == a.agent_id && b.resource_id == a.resource_id)
                .map(|b| b.quality)
                .unwrap_or(0.0))
            .sum();

        let certificate = ClearingCertificate {
            allocations,
            total_welfare,
            kkt_multipliers: kkt,
            input_hash: hash_bids(&bids),
            iterations: self.max_iterations,
            converged: true,
        };

        Ok(ComposeResult {
            composed: Signal::new(Kind::ClearingCertificate,
                serde_json::to_value(&certificate)?),
            accepted: inputs.iter().map(|s| s.id).collect(),
            budget_used: 0,
        })
    }
}

impl QpClearingCell {
    /// Bisection solver for the QP dual. O(80 * N) where N = number of bids.
    ///
    /// The dual problem has a single scalar parameter (the Lagrange multiplier
    /// for the total budget constraint). Bisection finds the optimal multiplier
    /// in 80 iterations to 1e-8 precision.
    fn solve_qp(
        &self,
        bids: &[ClearingBid],
    ) -> Result<(Vec<ClearingAllocation>, KktCertificate)> {
        let n = bids.len();

        // Bisection bounds for the dual variable.
        let mut lo = 0.0_f64;
        let mut hi = bids.iter().map(|b| b.quality).fold(0.0, f64::max) * 2.0;

        let mut best_allocations = Vec::new();

        for _iter in 0..self.max_iterations {
            let mid = (lo + hi) / 2.0;

            // For each bid, compute allocation at this dual value.
            let allocations: Vec<ClearingAllocation> = bids.iter()
                .map(|bid| {
                    // x_ij = max(0, (q_ij - mid * p_ij) / (2 * lambda * p_ij^2))
                    let numerator = bid.quality - mid * bid.price as f64;
                    let denominator = 2.0 * self.lambda * (bid.price as f64).powi(2);
                    let fraction = if denominator > 0.0 {
                        (numerator / denominator).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };

                    ClearingAllocation {
                        agent_id: bid.agent_id,
                        resource_id: bid.resource_id.clone(),
                        fraction,
                        price: (fraction * bid.price as f64) as u64,
                    }
                })
                .collect();

            // Check total budget constraint.
            let total_spend: f64 = allocations.iter()
                .map(|a| a.fraction * bids.iter()
                    .find(|b| b.agent_id == a.agent_id && b.resource_id == a.resource_id)
                    .map(|b| b.price as f64)
                    .unwrap_or(0.0))
                .sum();

            let total_budget: f64 = bids.iter().map(|b| b.budget as f64).sum::<f64>();

            if total_spend > total_budget {
                lo = mid; // tighten budget
            } else {
                hi = mid; // relax budget
                best_allocations = allocations;
            }

            if (hi - lo) < self.tolerance {
                break;
            }
        }

        // Compute KKT certificate from the solution.
        let kkt = self.compute_kkt_certificate(bids, &best_allocations, (lo + hi) / 2.0);

        Ok((best_allocations, kkt))
    }

    /// Compute KKT conditions for on-chain verification.
    /// The verifier checks these conditions hold without re-solving.
    fn compute_kkt_certificate(
        &self,
        bids: &[ClearingBid],
        allocations: &[ClearingAllocation],
        dual: f64,
    ) -> KktCertificate {
        let mut slackness_residuals = Vec::new();

        // Complementary slackness: for each allocation,
        // either x_ij = 0 or the gradient condition holds.
        for (alloc, bid) in allocations.iter().zip(bids.iter()) {
            let gradient = bid.quality - dual * bid.price as f64
                - 2.0 * self.lambda * alloc.fraction * (bid.price as f64).powi(2);
            let residual = alloc.fraction * gradient.abs();
            slackness_residuals.push(residual);
        }

        let max_residual = slackness_residuals.iter()
            .copied()
            .fold(0.0, f64::max);

        KktCertificate {
            agent_duals: vec![dual; bids.len()],
            resource_duals: vec![], // computed per-resource
            budget_duals: vec![],   // computed per-agent
            slackness_residuals,
            max_residual,
        }
    }
}
```

---

## 4. Settlement as a Verify Cell

Settlement verifies that a clearing allocation is correct and executes the on-chain fund transfers. It is a **Verify Cell**: it takes a `ClearingCertificate` Signal and produces a `Verdict` indicating whether the certificate is valid.

### 4.1 On-Chain KKT Verification

The elegance of the QP + KKT approach: the off-chain solver does the expensive computation (O(80N) bisection), then produces a certificate. The on-chain verifier only needs to check the KKT conditions (O(N) arithmetic) -- proving the solution is optimal without re-solving.

```rust
/// Settlement Verify Cell.
/// Takes a ClearingCertificate, verifies KKT conditions,
/// and executes on-chain settlement if valid.
///
/// Invalid certificates trigger slashing of the submitter.
pub struct SettlementVerifyCell {
    /// Chain client for submitting settlement transactions.
    chain_client: Arc<ChainClient>,

    /// Maximum acceptable KKT residual.
    max_residual: f64,  // default: 1e-6

    /// Escrow contract address.
    escrow_contract: Address,
}

impl Cell for SettlementVerifyCell {
    fn name(&self) -> &str { "settlement-verify" }
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Verify] }
}

#[async_trait]
impl VerifyProtocol for SettlementVerifyCell {
    async fn verify_pre(
        &self,
        signal: &Signal,
        ctx: &CellContext,
    ) -> Result<Verdict> {
        let cert: ClearingCertificate = serde_json::from_value(signal.payload.clone())?;

        // Step 1: Verify solver convergence.
        if !cert.converged {
            return Ok(Verdict {
                passed: false,
                reward: 0.0,
                evidence: Evidence::SettlementFailed,
                reason: "solver did not converge".into(),
            });
        }

        // Step 2: Verify KKT conditions.
        if cert.kkt_multipliers.max_residual > self.max_residual {
            return Ok(Verdict {
                passed: false,
                reward: 0.0,
                evidence: Evidence::KktViolation {
                    max_residual: cert.kkt_multipliers.max_residual,
                    threshold: self.max_residual,
                },
                reason: format!(
                    "KKT residual {} exceeds threshold {}",
                    cert.kkt_multipliers.max_residual, self.max_residual,
                ),
            });
        }

        // Step 3: Verify complementary slackness.
        for (i, residual) in cert.kkt_multipliers.slackness_residuals.iter().enumerate() {
            if *residual > self.max_residual {
                return Ok(Verdict {
                    passed: false,
                    reward: 0.0,
                    evidence: Evidence::SlacknessViolation { index: i, residual: *residual },
                    reason: format!("slackness violation at allocation {}: {}", i, residual),
                });
            }
        }

        // Step 4: Verify input hash (tamper detection).
        // The certificate's input_hash must match the hash of the original bids.
        // This prevents a submitter from solving a different problem.
        let stored_bids = ctx.store().get::<Vec<ClearingBid>>(&cert.input_hash).await?;
        let recomputed_hash = hash_bids(&stored_bids);
        if recomputed_hash != cert.input_hash {
            return Ok(Verdict {
                passed: false,
                reward: 0.0,
                evidence: Evidence::InputTampered,
                reason: "input hash mismatch: bids were altered".into(),
            });
        }

        // Step 5: Verify individual allocations are feasible.
        for alloc in &cert.allocations {
            if alloc.fraction < 0.0 || alloc.fraction > 1.0 {
                return Ok(Verdict {
                    passed: false,
                    reward: 0.0,
                    evidence: Evidence::InfeasibleAllocation,
                    reason: format!("allocation fraction {} out of [0,1]", alloc.fraction),
                });
            }
        }

        Ok(Verdict {
            passed: true,
            reward: 1.0,
            evidence: Evidence::SettlementVerified {
                total_welfare: cert.total_welfare,
                allocation_count: cert.allocations.len(),
                max_residual: cert.kkt_multipliers.max_residual,
            },
            reason: "KKT conditions satisfied. Settlement valid.".into(),
        })
    }

    async fn verify_post(
        &self,
        _signal: &Signal,
        output: &Signal,
        _ctx: &CellContext,
    ) -> Result<Verdict> {
        // Post-verification: confirm the on-chain transaction succeeded.
        if let Some(tx_hash) = output.payload.get("tx_hash").and_then(|v| v.as_str()) {
            let receipt = self.chain_client.get_receipt(tx_hash).await?;
            if receipt.status == 1 {
                return Ok(Verdict::pass());
            }
        }
        Ok(Verdict::fail("settlement transaction failed on-chain"))
    }
}
```

### 4.2 Settlement Execution

After verification passes, the settlement executes: escrow releases funds according to the allocation.

```rust
/// Execute on-chain settlement after Verify passes.
async fn execute_settlement(
    chain: &ChainClient,
    escrow: Address,
    cert: &ClearingCertificate,
) -> Result<TxHash> {
    // Build the settlement calldata.
    let calldata = encode_settlement(
        &cert.allocations,
        &cert.kkt_multipliers,
        cert.input_hash,
    );

    // Submit to the escrow contract.
    // The on-chain contract re-verifies KKT conditions.
    // Invalid submissions are rejected and the submitter is slashed.
    let tx = chain.send_transaction(
        escrow,
        calldata,
        GasEstimate::from_allocation_count(cert.allocations.len()),
    ).await?;

    Ok(tx)
}
```

### 4.3 Slashing for Invalid Certificates

If a submitter sends a `ClearingCertificate` that fails on-chain KKT verification, their stake is slashed. This makes submitting invalid certificates economically irrational.

```
Valid certificate:    submitter receives settlement fee (0.1% of cleared volume).
Invalid certificate:  submitter's stake is slashed (100% of posted bond).
```

The asymmetry is intentional: the reward for honest submission is small but frequent; the penalty for fraud is total and immediate.

---

## 5. Dispute Resolution as a Pipeline of Escalating Verify Cells

Disputes are resolved through a **Pipeline** of Verify Cells, each more expensive and authoritative than the last. The Pipeline short-circuits: if an early stage resolves the dispute, later stages are skipped.

```
[OptimisticVerify] --unresolved--> [BondEscalationVerify] --unresolved--> [PeerJuryVerify] --unresolved--> [GovernanceVerify]
     Level 0                            Level 1                             Level 2                          Level 3
     24h auto-accept                    Bond doubles each round             5 random agents, R>0.7           500 NUNCHI bond, final
```

### 5.1 Level 0: Optimistic Acceptance (Verify Cell)

Most transactions are honest. Level 0 accepts the result after a 24-hour challenge window with no action required. If no challenge is filed, the Verdict passes.

```rust
/// Level 0: Optimistic acceptance.
/// Accept after 24h unless challenged.
pub struct OptimisticVerifyCell {
    /// Challenge window duration.
    challenge_window: Duration,  // default: 24 hours
}

impl Cell for OptimisticVerifyCell {
    fn name(&self) -> &str { "dispute-l0-optimistic" }
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Verify] }
}

#[async_trait]
impl VerifyProtocol for OptimisticVerifyCell {
    async fn verify_pre(
        &self,
        signal: &Signal,
        ctx: &CellContext,
    ) -> Result<Verdict> {
        let submitted_at: DateTime<Utc> = signal.created_at;
        let challenge_deadline = submitted_at + self.challenge_window;

        // Check if any challenge was filed.
        let challenges = ctx.store().query(StoreQuery {
            kind: Some(Kind::DisputeChallenge),
            tags: Some(vec![format!("target:{}", signal.content_hash)]),
            ..Default::default()
        }).await?;

        if challenges.is_empty() && Utc::now() > challenge_deadline {
            // No challenge within window: optimistic acceptance.
            return Ok(Verdict {
                passed: true,
                reward: 1.0,
                evidence: Evidence::OptimisticAcceptance {
                    window: self.challenge_window,
                    challenges_filed: 0,
                },
                reason: "No challenge within 24h. Accepted.".into(),
            });
        }

        if challenges.is_empty() {
            // Still within window: wait.
            return Ok(Verdict {
                passed: false,
                reward: 0.0,
                evidence: Evidence::PendingChallenge,
                reason: format!(
                    "Challenge window open until {}",
                    challenge_deadline.format("%Y-%m-%d %H:%M UTC")
                ),
            });
        }

        // Challenge filed: escalate to Level 1.
        Ok(Verdict {
            passed: false,
            reward: 0.0,
            evidence: Evidence::ChallengeReceived {
                challenge_count: challenges.len(),
            },
            reason: "Challenge filed. Escalating to bond-escalation.".into(),
        })
    }

    async fn verify_post(&self, _s: &Signal, _o: &Signal, _c: &CellContext) -> Result<Verdict> {
        Ok(Verdict::pass())
    }
}
```

### 5.2 Level 1: Bond-Escalating Challenge (Verify Cell)

Each round of challenge requires doubling the bond. This prices frivolous disputes out quickly while allowing genuine disputes to escalate.

```rust
/// Level 1: Bond-escalating challenge.
/// Each challenge round doubles the required bond.
///
/// Round 0: initial_bond (e.g., 10 NUNCHI)
/// Round 1: 20 NUNCHI
/// Round 2: 40 NUNCHI
/// Round 3: 80 NUNCHI (cap)
///
/// If the challenger stops posting bonds, the original claim stands.
/// If the claimant stops defending, the challenge succeeds.
pub struct BondEscalationVerifyCell {
    /// Initial bond amount.
    initial_bond: u64,    // default: 10 NUNCHI

    /// Maximum bond (cap to prevent infinite escalation).
    max_bond: u64,        // default: 80 NUNCHI

    /// Time limit per round.
    round_timeout: Duration,  // default: 48 hours

    /// Maximum rounds before escalating to Level 2.
    max_rounds: u32,  // default: 4
}

impl Cell for BondEscalationVerifyCell {
    fn name(&self) -> &str { "dispute-l1-bond-escalation" }
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Verify] }
}

#[async_trait]
impl VerifyProtocol for BondEscalationVerifyCell {
    async fn verify_pre(
        &self,
        signal: &Signal,
        ctx: &CellContext,
    ) -> Result<Verdict> {
        let dispute: DisputeState = serde_json::from_value(signal.payload.clone())?;

        // Current round determines required bond.
        let required_bond = self.bond_for_round(dispute.current_round);

        // Check if the responding party posted their bond.
        let response = ctx.store().query(StoreQuery {
            kind: Some(Kind::DisputeBond),
            tags: Some(vec![
                format!("dispute:{}", dispute.dispute_id),
                format!("round:{}", dispute.current_round),
            ]),
            ..Default::default()
        }).await?;

        if response.is_empty() {
            // No response within timeout: the non-responding party loses.
            if Utc::now() > dispute.round_deadline {
                let winner = if dispute.current_round % 2 == 0 {
                    "challenger" // even rounds: claimant must respond
                } else {
                    "claimant"   // odd rounds: challenger must respond
                };

                return Ok(Verdict {
                    passed: winner == "claimant",
                    reward: if winner == "claimant" { 1.0 } else { 0.0 },
                    evidence: Evidence::BondTimeout {
                        round: dispute.current_round,
                        winner: winner.into(),
                    },
                    reason: format!("Round {} timed out. {} wins.", dispute.current_round, winner),
                });
            }

            // Still waiting for response.
            return Ok(Verdict {
                passed: false,
                reward: 0.0,
                evidence: Evidence::WaitingForBond {
                    round: dispute.current_round,
                    required_bond,
                },
                reason: format!("Waiting for round {} bond ({})", dispute.current_round, required_bond),
            });
        }

        // Response posted. If max rounds reached, escalate to Level 2.
        if dispute.current_round >= self.max_rounds {
            return Ok(Verdict {
                passed: false,
                reward: 0.0,
                evidence: Evidence::MaxRoundsReached,
                reason: "Bond escalation exhausted. Escalating to peer jury.".into(),
            });
        }

        // Advance to next round.
        Ok(Verdict {
            passed: false,
            reward: 0.0,
            evidence: Evidence::RoundAdvanced {
                round: dispute.current_round + 1,
                next_bond: self.bond_for_round(dispute.current_round + 1),
            },
            reason: format!("Advancing to round {}", dispute.current_round + 1),
        })
    }

    async fn verify_post(&self, _s: &Signal, _o: &Signal, _c: &CellContext) -> Result<Verdict> {
        Ok(Verdict::pass())
    }
}

impl BondEscalationVerifyCell {
    fn bond_for_round(&self, round: u32) -> u64 {
        let bond = self.initial_bond * 2u64.pow(round);
        bond.min(self.max_bond)
    }
}
```

### 5.3 Level 2: Peer Jury (Verify Cell)

Five random agents with reputation > 0.7 vote on the dispute. Weighted by reputation. Majority wins.

```rust
/// Level 2: Peer jury.
/// 5 randomly selected agents with R > 0.7 vote on the dispute.
/// Votes weighted by reputation. Majority (>50% weight) wins.
pub struct PeerJuryVerifyCell {
    /// Number of jurors.
    jury_size: usize,     // default: 5

    /// Minimum reputation to serve as juror.
    min_reputation: f64,  // default: 0.7

    /// Voting window.
    voting_window: Duration,  // default: 72 hours

    /// Reputation registry for selecting jurors.
    reputation: Arc<ReputationClient>,
}

impl Cell for PeerJuryVerifyCell {
    fn name(&self) -> &str { "dispute-l2-peer-jury" }
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Verify] }
}

#[async_trait]
impl VerifyProtocol for PeerJuryVerifyCell {
    async fn verify_pre(
        &self,
        signal: &Signal,
        ctx: &CellContext,
    ) -> Result<Verdict> {
        let dispute: DisputeState = serde_json::from_value(signal.payload.clone())?;

        // Select jury if not yet selected.
        let jury = self.select_jury(&dispute, ctx).await?;

        // Collect votes.
        let votes = ctx.store().query(StoreQuery {
            kind: Some(Kind::JuryVote),
            tags: Some(vec![format!("dispute:{}", dispute.dispute_id)]),
            ..Default::default()
        }).await?;

        // Check if voting is complete (all jurors voted or window expired).
        let voting_complete = votes.len() >= jury.len()
            || Utc::now() > dispute.jury_deadline.unwrap_or(Utc::now());

        if !voting_complete {
            return Ok(Verdict {
                passed: false,
                reward: 0.0,
                evidence: Evidence::JuryDeliberating {
                    votes_cast: votes.len(),
                    total_jurors: jury.len(),
                },
                reason: format!("{}/{} jurors voted", votes.len(), jury.len()),
            });
        }

        // Tally weighted votes.
        let mut weight_for: f64 = 0.0;
        let mut weight_against: f64 = 0.0;

        for vote in &votes {
            let juror_id: AgentId = vote.payload.get("juror_id")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default();
            let in_favor: bool = vote.payload.get("in_favor")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let weight = self.reputation.get_score(juror_id).await.unwrap_or(0.7);

            if in_favor {
                weight_for += weight;
            } else {
                weight_against += weight;
            }
        }

        let total_weight = weight_for + weight_against;
        let challenger_wins = weight_for > total_weight / 2.0;

        Ok(Verdict {
            passed: !challenger_wins, // passed = original claim stands
            reward: if challenger_wins { 0.0 } else { 1.0 },
            evidence: Evidence::JuryVerdict {
                weight_for,
                weight_against,
                juror_count: votes.len(),
            },
            reason: format!(
                "Jury verdict: {:.1}% in favor of challenge",
                weight_for / total_weight * 100.0,
            ),
        })
    }

    async fn verify_post(&self, _s: &Signal, _o: &Signal, _c: &CellContext) -> Result<Verdict> {
        Ok(Verdict::pass())
    }
}

impl PeerJuryVerifyCell {
    /// Select jury members. Random from eligible pool, excluding
    /// the disputing parties and their known associates.
    async fn select_jury(
        &self,
        dispute: &DisputeState,
        ctx: &CellContext,
    ) -> Result<Vec<AgentId>> {
        let eligible = self.reputation
            .get_agents_above_threshold(self.min_reputation)
            .await?;

        // Exclude parties to the dispute.
        let excluded: HashSet<AgentId> = [
            dispute.claimant_id,
            dispute.challenger_id,
        ].into_iter().collect();

        let candidates: Vec<AgentId> = eligible.into_iter()
            .filter(|id| !excluded.contains(id))
            .collect();

        // Random selection without replacement.
        let mut rng = StdRng::from_entropy();
        let selected: Vec<AgentId> = candidates.choose_multiple(&mut rng, self.jury_size)
            .cloned()
            .collect();

        Ok(selected)
    }
}
```

### 5.4 Level 3: Governance (Verify Cell)

The final appeal. Requires a 500 NUNCHI bond. Decision is final and on-chain.

```rust
/// Level 3: Governance.
/// Final appeal. 500 NUNCHI bond. Decision is immutable and on-chain.
pub struct GovernanceVerifyCell {
    /// Bond required to invoke governance.
    governance_bond: u64,  // default: 500 NUNCHI

    /// Governance contract address.
    governance_contract: Address,

    /// Voting period.
    voting_period: Duration,  // default: 7 days
}

impl Cell for GovernanceVerifyCell {
    fn name(&self) -> &str { "dispute-l3-governance" }
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Verify] }
}
```

### 5.5 Pipeline Assembly

```rust
/// Assemble the dispute resolution Pipeline.
/// Each Verify Cell can resolve the dispute. If it doesn't,
/// the Pipeline short-circuits to the next level.
fn build_dispute_pipeline(config: &DisputeConfig) -> PipelineGraph {
    PipelineGraph::new("dispute-resolution")
        .stage(OptimisticVerifyCell::new(config.challenge_window))
        .stage(BondEscalationVerifyCell::new(
            config.initial_bond,
            config.max_bond,
            config.round_timeout,
        ))
        .stage(PeerJuryVerifyCell::new(
            config.jury_size,
            config.min_reputation,
            config.voting_window,
        ))
        .stage(GovernanceVerifyCell::new(
            config.governance_bond,
            config.governance_contract,
        ))
}
```

### 5.6 Dispute Resolution Cost Profile

| Level | Cost | Time | Who Pays | When |
|---|---|---|---|---|
| L0: Optimistic | Free | 24h | Nobody | >95% of transactions |
| L1: Bond escalation | 10-80 NUNCHI per round | 48h/round | Loser loses bond | ~4% of disputes |
| L2: Peer jury | Juror time (compensated from losing bond) | 72h | Loser | ~0.9% of disputes |
| L3: Governance | 500 NUNCHI bond | 7 days | Loser | <0.1% of disputes |

The escalating cost structure ensures that the vast majority of disputes are resolved cheaply (L0/L1), while genuine disagreements can reach authoritative resolution (L2/L3).

---

## 6. Knowledge Attestation Types

Knowledge attestation connects payments and settlement to the knowledge registry ([22-REGISTRIES](../../unified/22-REGISTRIES.md) S4). Each attestation type is a Score Cell that produces a weighted evidence Signal.

```rust
/// The four attestation types for knowledge claims.
/// Each is a Score Cell variant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AttestationType {
    /// Another agent confirms the same finding independently.
    /// Weight: 1.0x base.
    Confirmation {
        confirming_agent: AgentId,
        evidence_hash: [u8; 32],
    },

    /// An agent independently reproduces the result via a different method.
    /// Weight: 2.0x base (strongest non-challenge attestation).
    IndependentVerification {
        verifying_agent: AgentId,
        method_description: String,
        evidence_hash: [u8; 32],
    },

    /// The knowledge was used in practice and produced the expected outcome.
    /// Weight: 1.5x base.
    UsageValidation {
        using_agent: AgentId,
        context: String,
        outcome: UsageOutcome,
    },

    /// Counter-evidence that contradicts the knowledge claim.
    /// Triggers dispute resolution (S5).
    Challenge {
        challenger: AgentId,
        counter_evidence_hash: [u8; 32],
        reason: String,
        bond_amount: u64,
    },
}

pub enum UsageOutcome {
    /// The knowledge was correct and useful.
    Positive { gate_pass_rate: f64 },
    /// The knowledge was correct but not useful for this context.
    Neutral,
    /// The knowledge was incorrect or misleading.
    Negative { failure_description: String },
}
```

---

## 7. Configuration

```toml
[payments]
# Default payment protocol.
default_protocol = "x402"

# x402 settlement.
[payments.x402]
max_amount_per_request = 1000    # USDC base units
batch_size = 100
settle_interval_secs = 600

# State channels.
[payments.state_channel]
dispute_window_blocks = 100
default_deposit = 10000

# Streaming.
[payments.stream]
buffer_seconds = 3600
min_flow_rate = 1               # wei/sec

# ISFR.
[payments.isfr]
min_total_weight = 10.0
min_claims = 3

# QP Clearing.
[payments.clearing]
lambda = 0.01
max_iterations = 80
tolerance = 1e-8
max_residual = 1e-6

# Dispute resolution.
[payments.disputes]
challenge_window_secs = 86400     # 24h
initial_bond = 10
max_bond = 80
round_timeout_secs = 172800       # 48h
jury_size = 5
min_jury_reputation = 0.7
governance_bond = 500
```

---

## What This Enables

1. **Swappable payment mechanisms**: Because x402, state channels, and streaming are all Connect Cells implementing the same protocol, any consumer of paid resources can switch between them without code changes. The Route protocol selects the optimal payment mechanism based on expected usage pattern.

2. **Composable settlement verification**: The KKT certificate pattern separates the expensive computation (off-chain QP solving) from the cheap verification (on-chain arithmetic). This is the same pattern as ZK proofs -- prove once, verify cheaply -- applied to economic coordination.

3. **Graduated dispute resolution**: The Pipeline of escalating Verify Cells ensures that trivial disputes are cheap and fast (24h optimistic acceptance) while serious disputes can reach authoritative resolution (governance). The bond-doubling mechanism prices out frivolous challenges at the economic layer.

4. **Trustless fact aggregation**: The ISFR Score Cell's sqrt-stake weighting prevents plutocratic capture while still weighting claims by skin-in-the-game. Combined with calibration scores from predict-publish-correct history, this creates a credibly neutral oracle that improves as agents build track records.

5. **End-to-end payment flow**: From HTTP 402 handshake through off-chain transacting to on-chain settlement, the entire flow decomposes into standard Cells. Each Cell can be monitored (Observe), tested (Verify), and learned from (predict-publish-correct Loop) using the same infrastructure as every other subsystem.

---

## Feedback Loops

- **Settlement Verdicts -> Reputation**: Valid settlement submissions earn positive reputation attestations for the submitter. Invalid submissions (KKT violation) result in slashing and negative attestation. This creates an incentive for honest clearing.

- **Dispute Outcomes -> Calibration**: When a dispute resolves, the winning party's calibration score improves and the losing party's decreases. Over time, agents with accurate claims build calibration that amplifies their ISFR weight.

- **Payment Protocol Usage -> Route Learning**: The Route Cell that selects payment protocols observes the outcome of each choice (gas cost, settlement success, latency). Via predict-publish-correct, it learns which protocol works best for which usage patterns.

- **ISFR Consensus -> Price Feeds**: ISFR consensus values feed into price oracles used by the clearing system. The clearing system produces settlement outcomes that attest to the correctness of the prices. This creates a circular validation: prices inform clearing, clearing validates prices.

- **Jury Verdicts -> Jury Selection**: Jurors who vote with the majority build reputation; those who consistently dissent may be excluded from future juries. This creates convergence pressure toward accurate dispute resolution (but risks groupthink -- see Open Questions).

---

## Open Questions

1. **Jury independence**: The peer jury selects 5 random agents with R > 0.7. If the high-reputation pool is small, jurors may know each other or have correlated incentives. Should there be a diversity constraint (e.g., no two jurors from the same capability domain)?

2. **Bond-escalation cap**: The 80 NUNCHI cap on bond escalation was chosen to prevent infinite escalation, but it may be too low for high-value disputes. Should the cap scale with the disputed amount?

3. **Stream payment underflow**: When a streaming payment's balance reaches the buffer amount, the stream should emit a warning (HealthStatus::Degraded). But the buffer exists to cover the stop-stream transaction gas cost. If gas prices spike, the buffer may be insufficient. Should the buffer dynamically adjust based on gas prices?

4. **QP solver numerical stability**: The bisection solver assumes the dual function is monotone, which holds for the regularized QP. If lambda is too small, the problem becomes nearly linear and bisection may oscillate. Should there be a minimum lambda enforced by the system?

5. **Cross-protocol settlement**: An agent may owe payments across multiple protocols (some x402, some state channel, some stream). Settling these atomically would reduce gas costs but adds complexity. Is there a Compose Cell that batches cross-protocol settlements?

6. **ISFR gaming via calibration**: An agent could build calibration by submitting easy, obvious claims, then leverage high calibration to influence a contested claim. The sqrt-stake weighting partially mitigates this (high stake required regardless of calibration), but should there be domain-specific calibration scores?
