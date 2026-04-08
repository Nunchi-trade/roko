//! [`WalletGate`] — §33.4.10: a [`Gate`] that checks balance / nonce
//! preconditions before an agent signs a tx.
//!
//! The gate reads a [`TxRequest`] encoded as JSON in the signal body,
//! computes the wei the wallet must cover (`value + gas_limit * max_fee_per_gas`),
//! and compares it to the wallet's on-chain balance. Optionally it also
//! enforces that the pinned nonce on the tx matches the wallet's current
//! nonce — catching nonce gaps *before* a signed tx leaves the process.
//!
//! The `require_allowance_for` config field is reserved for a future ERC-20
//! allowance check (§33.4.12 Permit2 work). It is accepted today and exposed
//! via [`WalletGateConfig`] so callers can wire it in without changing their
//! type, but this gate does not yet perform the allowance call — the check
//! will be added alongside the Permit2 helpers.

use async_trait::async_trait;
use roko_core::{Body, Context, Signal, traits::Gate, verdict::Verdict};
use serde::Deserialize;
use std::sync::Arc;
use std::time::Instant;

use crate::{ChainClient, ChainWallet, TxRequest};

/// Convert `started.elapsed()` to a `u64` millisecond count, saturating at
/// `u64::MAX` to satisfy clippy's truncation lint (durations that long are
/// already broken by any definition of "gate latency").
fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// Configuration for [`WalletGate`].
#[derive(Clone, Copy, Debug)]
pub struct WalletGateConfig {
    /// Reject the tx if the wallet balance drops below this wei amount
    /// **after** paying for the tx. Set to `0` to disable the floor.
    pub min_balance_wei: u128,
    /// Optional ERC-20 token address the wallet must have an allowance for.
    ///
    /// Reserved for the §33.4.12 Permit2 work — accepted by the config
    /// today but not yet enforced by [`WalletGate::verify`].
    pub require_allowance_for: Option<[u8; 20]>,
    /// When `true`, a pinned nonce on the tx must match the wallet's
    /// current nonce exactly.
    pub strict_nonce: bool,
}

impl Default for WalletGateConfig {
    fn default() -> Self {
        Self {
            min_balance_wei: 0,
            require_allowance_for: None,
            strict_nonce: true,
        }
    }
}

/// A [`Gate`] that verifies a wallet is ready to sign the tx encoded in the
/// input signal.
///
/// Holds `Arc<dyn ChainWallet>` + `Arc<dyn ChainClient>` so multiple gates
/// / agents can share the same wallet handle across tasks. The gate itself
/// is `Clone` — cloning only bumps refcounts.
#[derive(Clone)]
pub struct WalletGate {
    wallet: Arc<dyn ChainWallet>,
    #[allow(dead_code)] // reserved for future allowance / chain_id checks
    client: Arc<dyn ChainClient>,
    config: WalletGateConfig,
    name: String,
}

/// Classification of a single wallet readiness check.
///
/// Produced by [`WalletGate::check`]; the `verify` entry point turns these
/// into [`Verdict`]s.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WalletCheck {
    /// Balance is sufficient to cover the requested amount.
    BalanceOk {
        /// Wallet balance, in wei.
        have: u128,
        /// Required amount, in wei (value + gas).
        need: u128,
    },
    /// Balance is below the required amount.
    InsufficientBalance {
        /// Wallet balance, in wei.
        have: u128,
        /// Required amount, in wei (value + gas).
        need: u128,
    },
    /// A pinned tx nonce did not match the wallet's next nonce.
    NonceGap {
        /// Nonce the wallet expects next.
        expected: u64,
        /// Nonce actually seen on the tx.
        got: u64,
    },
    /// The check could not be completed (e.g. backend offline).
    Unsupported {
        /// Human-readable reason the check was skipped.
        reason: String,
    },
}

impl WalletGate {
    /// Build a new `WalletGate` bound to a wallet + client pair.
    #[must_use]
    pub fn new(
        wallet: Arc<dyn ChainWallet>,
        client: Arc<dyn ChainClient>,
        config: WalletGateConfig,
    ) -> Self {
        Self {
            wallet,
            client,
            config,
            name: "wallet_gate".to_string(),
        }
    }

    /// Build a new `WalletGate` with a custom name (useful when multiple
    /// wallet gates run in parallel and their verdicts need to be told apart
    /// in logs / metrics).
    #[must_use]
    pub fn with_name(
        wallet: Arc<dyn ChainWallet>,
        client: Arc<dyn ChainClient>,
        config: WalletGateConfig,
        name: impl Into<String>,
    ) -> Self {
        Self {
            wallet,
            client,
            config,
            name: name.into(),
        }
    }

    /// Inspect the underlying wallet balance and classify against the
    /// requested `needed_wei`. Returns a [`WalletCheck`] describing the
    /// outcome. Used internally by [`Gate::verify`]; exposed publicly so
    /// callers can pre-flight checks without constructing a `Signal`.
    pub async fn check(&self, needed_wei: u128) -> WalletCheck {
        let balance = match self.wallet.balance(None).await {
            Ok(b) => b,
            Err(e) => {
                return WalletCheck::Unsupported {
                    reason: format!("balance lookup failed: {e}"),
                };
            }
        };

        // Enforce the min-balance floor: balance after paying for the tx
        // must be ≥ min_balance_wei.
        let total_need = needed_wei.saturating_add(self.config.min_balance_wei);
        if balance < total_need {
            return WalletCheck::InsufficientBalance {
                have: balance,
                need: total_need,
            };
        }
        WalletCheck::BalanceOk {
            have: balance,
            need: total_need,
        }
    }

    /// Compute the wei required to cover a tx: `value + gas_limit * max_fee_per_gas`.
    /// Gas cost only contributes when both `gas_limit` and `max_fee_per_gas`
    /// are pinned on the tx (otherwise the wallet will estimate at sign time).
    /// Saturating arithmetic keeps the computation in `u128` without panic on
    /// overflow — overflow is surfaced downstream as "insufficient balance".
    fn needed_wei(tx: &TxRequest) -> u128 {
        let gas_cost = match (tx.gas_limit, tx.max_fee_per_gas) {
            (Some(g), Some(fee)) => u128::from(g).saturating_mul(fee),
            _ => 0,
        };
        tx.value.saturating_add(gas_cost)
    }
}

/// Internal JSON shape we deserialize from the signal body. Kept separate
/// from [`TxRequest`] because the latter is not `Deserialize` (its fields
/// use backend-agnostic primitives that we don't want serde tied to).
#[derive(Debug, Deserialize)]
struct TxRequestJson {
    #[serde(default)]
    to: Option<String>,
    #[serde(default)]
    from: Option<String>,
    #[serde(default)]
    value: Option<u128>,
    #[serde(default)]
    gas_limit: Option<u64>,
    #[serde(default)]
    max_fee_per_gas: Option<u128>,
    #[serde(default)]
    max_priority_fee_per_gas: Option<u128>,
    #[serde(default)]
    nonce: Option<u64>,
    // `data` is accepted but ignored by the wallet gate — balance/nonce
    // decisions do not depend on calldata.
    #[serde(default)]
    #[allow(dead_code)]
    data: Option<serde_json::Value>,
}

impl From<TxRequestJson> for TxRequest {
    fn from(j: TxRequestJson) -> Self {
        Self {
            to: j.to,
            from: j.from,
            value: j.value.unwrap_or(0),
            data: Vec::new(),
            gas_limit: j.gas_limit,
            max_fee_per_gas: j.max_fee_per_gas,
            max_priority_fee_per_gas: j.max_priority_fee_per_gas,
            nonce: j.nonce,
        }
    }
}

/// Parse a `TxRequest` out of a signal body. Returns the parse error string
/// for a failing verdict when the body is not a recognised JSON shape.
pub(crate) fn parse_tx_from_signal(signal: &Signal) -> Result<TxRequest, String> {
    let json: TxRequestJson = match &signal.body {
        Body::Json(v) => serde_json::from_value(v.clone())
            .map_err(|e| format!("body json does not match TxRequest: {e}"))?,
        Body::Text(s) => serde_json::from_str(s)
            .map_err(|e| format!("body text is not valid TxRequest json: {e}"))?,
        Body::Bytes(_) => {
            return Err("bytes body is not supported; use text/json".to_string());
        }
        Body::Empty => {
            return Err("empty body; expected TxRequest json".to_string());
        }
    };
    Ok(json.into())
}

#[async_trait]
impl Gate for WalletGate {
    async fn verify(&self, input: &Signal, _ctx: &Context) -> Verdict {
        let started = Instant::now();

        let tx = match parse_tx_from_signal(input) {
            Ok(t) => t,
            Err(reason) => {
                return Verdict::fail(self.name.clone(), reason).with_duration(elapsed_ms(started));
            }
        };

        // Balance check.
        let needed = Self::needed_wei(&tx);
        let verdict = match self.check(needed).await {
            WalletCheck::BalanceOk { have, need } => Verdict::pass(self.name.clone())
                .with_detail(format!("balance_ok have={have} need={need}")),
            WalletCheck::InsufficientBalance { have, need } => Verdict::fail(
                self.name.clone(),
                format!("insufficient balance: have {have}, need {need}"),
            ),
            WalletCheck::NonceGap { expected, got } => Verdict::fail(
                self.name.clone(),
                format!("nonce gap: expected {expected}, got {got}"),
            ),
            WalletCheck::Unsupported { reason } => Verdict::fail(
                self.name.clone(),
                format!("wallet check unsupported: {reason}"),
            ),
        };

        if !verdict.passed {
            return verdict.with_duration(elapsed_ms(started));
        }

        // Strict nonce check: only when the tx pins a nonce.
        if self.config.strict_nonce {
            if let Some(got) = tx.nonce {
                match self.wallet.nonce().await {
                    Ok(expected) if expected != got => {
                        return Verdict::fail(
                            self.name.clone(),
                            format!("nonce gap: expected {expected}, got {got}"),
                        )
                        .with_duration(elapsed_ms(started));
                    }
                    Ok(_) => {}
                    Err(e) => {
                        return Verdict::fail(
                            self.name.clone(),
                            format!("nonce lookup failed: {e}"),
                        )
                        .with_duration(elapsed_ms(started));
                    }
                }
            }
        }

        verdict.with_duration(elapsed_ms(started))
    }

    fn name(&self) -> &str {
        &self.name
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::{MockChainClient, MockChainWallet, paired_mocks};
    use roko_core::{Body, Context, Kind, Provenance, Signal};

    fn cfg() -> WalletGateConfig {
        WalletGateConfig::default()
    }

    fn tx_signal_json(v: serde_json::Value) -> Signal {
        Signal::builder(Kind::Transaction)
            .body(Body::Json(v))
            .provenance(Provenance::agent("alice"))
            .build()
    }

    fn make_gate(balance: u128) -> (WalletGate, MockChainWallet, MockChainClient) {
        let (client, wallet) = paired_mocks(balance);
        let gate = WalletGate::new(Arc::new(wallet.clone()), Arc::new(client.clone()), cfg());
        (gate, wallet, client)
    }

    #[tokio::test(flavor = "current_thread")]
    async fn check_passes_with_sufficient_balance() {
        let (gate, _w, _c) = make_gate(1_000);
        let result = gate.check(500).await;
        assert!(matches!(
            result,
            WalletCheck::BalanceOk {
                have: 1_000,
                need: 500
            }
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn check_fails_with_insufficient_balance() {
        let (gate, _w, _c) = make_gate(100);
        let result = gate.check(500).await;
        assert!(matches!(
            result,
            WalletCheck::InsufficientBalance {
                have: 100,
                need: 500
            }
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn check_respects_min_balance_floor() {
        let (client, wallet) = paired_mocks(1_000);
        let gate = WalletGate::new(
            Arc::new(wallet),
            Arc::new(client),
            WalletGateConfig {
                min_balance_wei: 400,
                ..WalletGateConfig::default()
            },
        );
        // Need 700, balance 1000 → after tx = 300 which is < 400 floor.
        let result = gate.check(700).await;
        assert!(matches!(
            result,
            WalletCheck::InsufficientBalance {
                have: 1_000,
                need: 1_100
            }
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn check_strict_nonce_flags_gap() {
        // The wallet sits at nonce=0; the signal pins nonce=5 → verify should
        // fail. `check` itself doesn't touch nonces; this exercises verify.
        let (client, wallet) = paired_mocks(1_000);
        let gate = WalletGate::new(Arc::new(wallet), Arc::new(client), cfg());
        let signal = tx_signal_json(serde_json::json!({
            "value": 100,
            "nonce": 5,
        }));
        let verdict = gate.verify(&signal, &Context::now()).await;
        assert!(!verdict.passed);
        assert!(verdict.reason.contains("nonce gap"));
        assert!(verdict.reason.contains("expected 0"));
        assert!(verdict.reason.contains("got 5"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn verify_pass_on_well_formed_tx_signal() {
        let (gate, _w, _c) = make_gate(1_000_000);
        let signal = tx_signal_json(serde_json::json!({
            "to": "0xabc",
            "value": 1000,
            "gas_limit": 21000,
            "max_fee_per_gas": 10,
        }));
        let verdict = gate.verify(&signal, &Context::now()).await;
        assert!(verdict.passed, "verdict should pass: {verdict:?}");
        assert_eq!(verdict.gate, "wallet_gate");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn verify_fail_with_insufficient_balance_signal() {
        let (gate, _w, _c) = make_gate(50);
        let signal = tx_signal_json(serde_json::json!({
            "value": 1000,
        }));
        let verdict = gate.verify(&signal, &Context::now()).await;
        assert!(!verdict.passed);
        assert!(verdict.reason.contains("insufficient balance"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn verify_includes_gas_cost_in_need() {
        // Balance 500, value 0, but gas_limit × fee = 21000 × 100 = 2_100_000
        // → should fail even though value is 0.
        let (gate, _w, _c) = make_gate(500);
        let signal = tx_signal_json(serde_json::json!({
            "value": 0,
            "gas_limit": 21000,
            "max_fee_per_gas": 100,
        }));
        let verdict = gate.verify(&signal, &Context::now()).await;
        assert!(!verdict.passed);
        assert!(verdict.reason.contains("insufficient balance"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn verify_fail_on_non_json_body() {
        let (gate, _w, _c) = make_gate(1_000);
        let signal = Signal::builder(Kind::Transaction)
            .body(Body::Bytes(vec![1, 2, 3]))
            .provenance(Provenance::agent("alice"))
            .build();
        let verdict = gate.verify(&signal, &Context::now()).await;
        assert!(!verdict.passed);
        assert!(verdict.reason.contains("bytes body is not supported"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn verify_fail_on_empty_body() {
        let (gate, _w, _c) = make_gate(1_000);
        let signal = Signal::builder(Kind::Transaction)
            .body(Body::Empty)
            .provenance(Provenance::agent("alice"))
            .build();
        let verdict = gate.verify(&signal, &Context::now()).await;
        assert!(!verdict.passed);
        assert!(verdict.reason.contains("empty body"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn verify_fail_on_malformed_tx_request() {
        let (gate, _w, _c) = make_gate(1_000);
        // `value` must be numeric; a string value fails to deserialize.
        let signal = tx_signal_json(serde_json::json!({
            "value": "not-a-number",
        }));
        let verdict = gate.verify(&signal, &Context::now()).await;
        assert!(!verdict.passed);
        assert!(
            verdict
                .reason
                .contains("body json does not match TxRequest")
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn verify_accepts_text_body_json() {
        let (gate, _w, _c) = make_gate(10_000);
        let signal = Signal::builder(Kind::Transaction)
            .body(Body::Text(r#"{"value": 100}"#.to_string()))
            .provenance(Provenance::agent("alice"))
            .build();
        let verdict = gate.verify(&signal, &Context::now()).await;
        assert!(verdict.passed);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn verify_strict_nonce_off_ignores_pinned_nonce() {
        let (client, wallet) = paired_mocks(1_000);
        let gate = WalletGate::new(
            Arc::new(wallet),
            Arc::new(client),
            WalletGateConfig {
                strict_nonce: false,
                ..WalletGateConfig::default()
            },
        );
        let signal = tx_signal_json(serde_json::json!({
            "value": 100,
            "nonce": 99,
        }));
        let verdict = gate.verify(&signal, &Context::now()).await;
        assert!(verdict.passed, "strict_nonce=false must skip nonce check");
    }

    #[test]
    fn name_returns_wallet_gate() {
        let client = MockChainClient::local();
        let wallet = MockChainWallet::funded(0);
        let gate = WalletGate::new(Arc::new(wallet), Arc::new(client), cfg());
        assert_eq!(gate.name(), "wallet_gate");
    }

    #[test]
    fn with_name_overrides() {
        let client = MockChainClient::local();
        let wallet = MockChainWallet::funded(0);
        let gate =
            WalletGate::with_name(Arc::new(wallet), Arc::new(client), cfg(), "arb_wallet_gate");
        assert_eq!(gate.name(), "arb_wallet_gate");
    }
}
