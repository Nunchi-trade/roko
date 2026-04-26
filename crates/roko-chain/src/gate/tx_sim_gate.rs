//! [`TxSimGate`] — §33.4.11: a [`Verify`] that runs a planned tx through a
//! simulator before the agent signs it.
//!
//! `TxSimGate` is deliberately decoupled from any particular simulator. Users
//! pass any [`TxSimulator`] impl: a thin wrapper over mirage-rs's
//! `SimulationGate`, an alloy `eth_call`, a revm-based sandbox, or the
//! [`MockTxSimulator`] provided here for tests.
//!
//! # Verdict rules
//!
//! * If the simulator errors → `Verdict::fail` with the error string.
//! * If `require_success` is `true` and the sim reverts → `Verdict::fail`.
//! * If `gas_used` exceeds `gas_limit * (1 - gas_buffer_pct / 100)` → fail.
//! * Otherwise → `Verdict::pass` with gas usage encoded in `detail`.
//!
//! The gate reads the `TxRequest` out of the signal body the same way
//! [`WalletGate`](crate::WalletGate) does, so agents emit ONE signal that both
//! gates can verify.

use async_trait::async_trait;
use roko_core::{Context, Engram, traits::Verify, verdict::Verdict};
use std::sync::Arc;
use std::time::Instant;

use crate::{ChainError, TxRequest};

use super::wallet_gate::parse_tx_from_signal;

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// Outcome of simulating a single transaction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SimulationOutcome {
    /// `true` if the simulator thinks the tx would succeed.
    pub success: bool,
    /// Gas the simulator reports as consumed.
    pub gas_used: u64,
    /// Decoded revert reason (if the simulator can extract one).
    pub revert_reason: Option<String>,
}

impl SimulationOutcome {
    /// A successful outcome with the given gas usage.
    #[must_use]
    pub const fn ok(gas_used: u64) -> Self {
        Self {
            success: true,
            gas_used,
            revert_reason: None,
        }
    }

    /// A reverted outcome with a reason.
    #[must_use]
    pub fn reverted(gas_used: u64, reason: impl Into<String>) -> Self {
        Self {
            success: false,
            gas_used,
            revert_reason: Some(reason.into()),
        }
    }
}

/// Simulates a `TxRequest` without submitting it to the chain.
///
/// Implementations wrap a concrete simulator: mirage-rs's fork executor,
/// an alloy `eth_call`, a revm sandbox, or the [`MockTxSimulator`] for tests.
/// Roko's `TxSimGate` depends only on this trait — it never names a
/// particular simulator backend.
#[async_trait]
pub trait TxSimulator: Send + Sync {
    /// Simulate `tx`. Returns a [`SimulationOutcome`] on success, or a
    /// [`ChainError`] if the simulator itself failed (timeout, offline, …).
    async fn simulate(&self, tx: &TxRequest) -> Result<SimulationOutcome, ChainError>;
}

/// Configuration for [`TxSimGate`].
#[derive(Clone, Copy, Debug)]
pub struct TxSimGateConfig {
    /// Gas-buffer percentage, 0–50. The gate requires
    /// `gas_used ≤ gas_limit * (1 - buffer / 100)`. Set to `0` to disable.
    pub gas_buffer_pct: u8,
    /// When `true`, a reverted sim produces a failing verdict.
    pub require_success: bool,
}

impl Default for TxSimGateConfig {
    fn default() -> Self {
        Self {
            gas_buffer_pct: 10,
            require_success: true,
        }
    }
}

/// A [`Verify`] that simulates the planned tx and fails if the simulator reverts
/// or the tx would consume more than the allowed portion of its gas budget.
#[derive(Clone)]
pub struct TxSimGate {
    sim: Arc<dyn TxSimulator>,
    config: TxSimGateConfig,
    name: String,
}

impl TxSimGate {
    /// Build a new `TxSimGate` with the given simulator + config.
    #[must_use]
    pub fn new(sim: Arc<dyn TxSimulator>, config: TxSimGateConfig) -> Self {
        Self {
            sim,
            config,
            name: "tx_sim_gate".to_string(),
        }
    }

    /// Build a new `TxSimGate` with a custom name.
    #[must_use]
    pub fn with_name(
        sim: Arc<dyn TxSimulator>,
        config: TxSimGateConfig,
        name: impl Into<String>,
    ) -> Self {
        Self {
            sim,
            config,
            name: name.into(),
        }
    }

    /// Compute the upper bound on `gas_used` allowed by the configured buffer.
    /// Returns `None` if the tx did not pin a `gas_limit`, in which case the
    /// gate skips the buffer check (the wallet will set the limit at sign
    /// time and downstream sim-vs-limit enforcement is out of scope here).
    fn gas_ceiling(&self, tx: &TxRequest) -> Option<u64> {
        let limit = tx.gas_limit?;
        let buf = u64::from(self.config.gas_buffer_pct.min(50));
        let allowed_pct = 100u64.saturating_sub(buf);
        // gas_limit * allowed_pct / 100 — u64 math with saturation.
        let product = u128::from(limit).saturating_mul(u128::from(allowed_pct)) / 100u128;
        Some(u64::try_from(product).unwrap_or(u64::MAX))
    }
}

#[async_trait]
impl Verify for TxSimGate {
    async fn verify(&self, input: &Engram, _ctx: &Context) -> Verdict {
        let started = Instant::now();

        let tx = match parse_tx_from_signal(input) {
            Ok(t) => t,
            Err(reason) => {
                return Verdict::fail(self.name.clone(), reason).with_duration(elapsed_ms(started));
            }
        };

        let outcome = match self.sim.simulate(&tx).await {
            Ok(o) => o,
            Err(e) => {
                return Verdict::fail(self.name.clone(), format!("simulator error: {e}"))
                    .with_duration(elapsed_ms(started));
            }
        };

        let detail = format!(
            "gas_used={} success={} revert={}",
            outcome.gas_used,
            outcome.success,
            outcome.revert_reason.as_deref().unwrap_or("")
        );

        // Require-success check.
        if self.config.require_success && !outcome.success {
            let reason = outcome
                .revert_reason
                .as_deref()
                .unwrap_or("tx reverted during simulation");
            return Verdict::fail(self.name.clone(), format!("revert: {reason}"))
                .with_detail(detail)
                .with_duration(elapsed_ms(started));
        }

        // Gas-buffer check (only when both gas_limit and gas_buffer_pct are set).
        if self.config.gas_buffer_pct > 0 {
            if let Some(ceiling) = self.gas_ceiling(&tx) {
                if outcome.gas_used > ceiling {
                    return Verdict::fail(
                        self.name.clone(),
                        format!(
                            "gas over buffer: used {} > allowed {}",
                            outcome.gas_used, ceiling
                        ),
                    )
                    .with_detail(detail)
                    .with_duration(elapsed_ms(started));
                }
            }
        }

        Verdict::pass(self.name.clone())
            .with_detail(detail)
            .with_duration(elapsed_ms(started))
    }

    fn name(&self) -> &str {
        &self.name
    }
}

/// A canned [`TxSimulator`] for tests — returns the configured outcome on
/// every `simulate` call.
///
/// ```
/// use std::sync::Arc;
/// use roko_chain::{MockTxSimulator, SimulationOutcome, TxSimGate, TxSimGateConfig, TxSimulator};
///
/// let sim = MockTxSimulator { outcome: SimulationOutcome::ok(21_000) };
/// let gate = TxSimGate::new(Arc::new(sim), TxSimGateConfig::default());
/// let _ = gate;
/// ```
#[derive(Clone, Debug)]
pub struct MockTxSimulator {
    /// The outcome every `simulate` call returns.
    pub outcome: SimulationOutcome,
}

#[async_trait]
impl TxSimulator for MockTxSimulator {
    async fn simulate(&self, _tx: &TxRequest) -> Result<SimulationOutcome, ChainError> {
        Ok(self.outcome.clone())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use roko_core::{Body, Context, Engram, Kind, Provenance};

    #[derive(Clone, Debug)]
    struct FailingSimulator {
        message: String,
    }

    #[async_trait]
    impl TxSimulator for FailingSimulator {
        async fn simulate(&self, _tx: &TxRequest) -> Result<SimulationOutcome, ChainError> {
            Err(ChainError::Rpc(self.message.clone()))
        }
    }

    fn tx_signal_json(v: serde_json::Value) -> Engram {
        Engram::builder(Kind::Transaction)
            .body(Body::Json(v))
            .provenance(Provenance::agent("alice"))
            .build()
    }

    fn sim_gate(outcome: SimulationOutcome, cfg: TxSimGateConfig) -> TxSimGate {
        TxSimGate::new(Arc::new(MockTxSimulator { outcome }), cfg)
    }

    #[tokio::test(flavor = "current_thread")]
    async fn simulate_success_passes_gate() {
        let gate = sim_gate(SimulationOutcome::ok(21_000), TxSimGateConfig::default());
        let signal = tx_signal_json(serde_json::json!({
            "to": "0xabc",
            "value": 1,
            "gas_limit": 100_000,
        }));
        let verdict = gate.verify(&signal, &Context::now()).await;
        assert!(verdict.passed, "should pass: {verdict:?}");
        assert_eq!(verdict.gate, "tx_sim_gate");
        assert!(
            verdict
                .detail
                .as_deref()
                .unwrap()
                .contains("gas_used=21000")
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn simulate_revert_fails_with_require_success() {
        let gate = sim_gate(
            SimulationOutcome::reverted(50_000, "underflow"),
            TxSimGateConfig {
                require_success: true,
                gas_buffer_pct: 0,
            },
        );
        let signal = tx_signal_json(serde_json::json!({"value": 1}));
        let verdict = gate.verify(&signal, &Context::now()).await;
        assert!(!verdict.passed);
        assert!(verdict.reason.contains("revert"));
        assert!(verdict.reason.contains("underflow"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn simulate_revert_passes_without_require_success() {
        let gate = sim_gate(
            SimulationOutcome::reverted(50_000, "ignored"),
            TxSimGateConfig {
                require_success: false,
                gas_buffer_pct: 0,
            },
        );
        let signal = tx_signal_json(serde_json::json!({"value": 1}));
        let verdict = gate.verify(&signal, &Context::now()).await;
        assert!(
            verdict.passed,
            "revert OK when require_success=false: {verdict:?}"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn high_gas_used_over_buffer_fails_gate() {
        // 10% buffer → allowed = 100_000 * 0.9 = 90_000. gas_used 95_000 fails.
        let gate = sim_gate(
            SimulationOutcome::ok(95_000),
            TxSimGateConfig {
                require_success: true,
                gas_buffer_pct: 10,
            },
        );
        let signal = tx_signal_json(serde_json::json!({
            "value": 1,
            "gas_limit": 100_000,
        }));
        let verdict = gate.verify(&signal, &Context::now()).await;
        assert!(!verdict.passed);
        assert!(verdict.reason.contains("gas over buffer"));
        assert!(verdict.reason.contains("used 95000"));
        assert!(verdict.reason.contains("allowed 90000"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn gas_used_under_buffer_passes_gate() {
        // 10% buffer, gas_limit 100_000 → allowed 90_000. gas_used 80_000 passes.
        let gate = sim_gate(
            SimulationOutcome::ok(80_000),
            TxSimGateConfig {
                require_success: true,
                gas_buffer_pct: 10,
            },
        );
        let signal = tx_signal_json(serde_json::json!({
            "value": 1,
            "gas_limit": 100_000,
        }));
        let verdict = gate.verify(&signal, &Context::now()).await;
        assert!(verdict.passed, "verdict: {verdict:?}");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn gas_buffer_skipped_without_gas_limit() {
        // No gas_limit on tx → buffer check is skipped even if gas_used is huge.
        let gate = sim_gate(
            SimulationOutcome::ok(9_999_999),
            TxSimGateConfig {
                require_success: true,
                gas_buffer_pct: 10,
            },
        );
        let signal = tx_signal_json(serde_json::json!({"value": 1}));
        let verdict = gate.verify(&signal, &Context::now()).await;
        assert!(verdict.passed, "verdict: {verdict:?}");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn simulator_error_fails_gate() {
        let sim = Arc::new(FailingSimulator {
            message: "rpc down".to_string(),
        });
        let gate = TxSimGate::new(sim, TxSimGateConfig::default());
        let signal = tx_signal_json(serde_json::json!({"value": 1}));
        let verdict = gate.verify(&signal, &Context::now()).await;
        assert!(!verdict.passed);
        assert!(verdict.reason.contains("simulator error"));
        assert!(verdict.reason.contains("rpc down"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn verify_fail_on_non_json_body() {
        let gate = sim_gate(SimulationOutcome::ok(1), TxSimGateConfig::default());
        let signal = Engram::builder(Kind::Transaction)
            .body(Body::Bytes(vec![1, 2, 3]))
            .provenance(Provenance::agent("alice"))
            .build();
        let verdict = gate.verify(&signal, &Context::now()).await;
        assert!(!verdict.passed);
        assert!(verdict.reason.contains("bytes body is not supported"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn verify_fail_on_malformed_tx() {
        let gate = sim_gate(SimulationOutcome::ok(1), TxSimGateConfig::default());
        let signal = tx_signal_json(serde_json::json!({"value": "nope"}));
        let verdict = gate.verify(&signal, &Context::now()).await;
        assert!(!verdict.passed);
        assert!(
            verdict
                .reason
                .contains("body json does not match TxRequest")
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn mock_simulator_returns_configured_outcome() {
        let sim = MockTxSimulator {
            outcome: SimulationOutcome::reverted(42, "rev"),
        };
        let outcome = sim.simulate(&TxRequest::default()).await.unwrap();
        assert!(!outcome.success);
        assert_eq!(outcome.gas_used, 42);
        assert_eq!(outcome.revert_reason.as_deref(), Some("rev"));
    }

    #[test]
    fn name_returns_tx_sim_gate() {
        let gate = sim_gate(SimulationOutcome::ok(1), TxSimGateConfig::default());
        assert_eq!(gate.name(), "tx_sim_gate");
    }

    #[test]
    fn with_name_overrides_gate_name() {
        let sim = Arc::new(MockTxSimulator {
            outcome: SimulationOutcome::ok(1),
        });
        let gate = TxSimGate::with_name(sim, TxSimGateConfig::default(), "arb_sim");
        assert_eq!(gate.name(), "arb_sim");
    }

    #[test]
    fn simulation_outcome_builders() {
        let ok = SimulationOutcome::ok(21_000);
        assert!(ok.success);
        assert_eq!(ok.gas_used, 21_000);
        assert!(ok.revert_reason.is_none());
        let bad = SimulationOutcome::reverted(500, "boom");
        assert!(!bad.success);
        assert_eq!(bad.revert_reason.as_deref(), Some("boom"));
    }
}
