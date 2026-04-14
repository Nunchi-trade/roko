//! `SimulationGate`: a roko `Gate` impl that runs planned transactions through
//! mirage's fork and returns a `Verdict` telling the agent whether the tx would
//! succeed.
//!
//! # Engram shape
//!
//! Input signal body must be a JSON object matching the `TransactionRequest`
//! shape defined in mirage-rs (`from`, `to`, `gas`, `value`, `data`, ...).
//! Example:
//!
//! ```json
//! {
//!   "from": "0x1111111111111111111111111111111111111111",
//!   "to":   "0x2222222222222222222222222222222222222222",
//!   "data": "0x095ea7b3…",
//!   "gas":  "0x5208"
//! }
//! ```
//!
//! The gate reads the mirage fork at simulation time (read-only snapshot — does
//! not mutate fork state) and produces a `Verdict` with:
//!   - `passed`: true iff `ExecutionResult.success`
//!   - `score`: 1.0 on success, 0.0 on revert
//!   - `detail`: JSON summary with gas_used and hex-encoded output

use async_trait::async_trait;
use roko_core::{Body, Context, Engram, traits::Gate, verdict::Verdict};
use serde::Deserialize;

use crate::{
    MirageError, TransactionRequest,
    fork::{EvmExecutor, MirageFork},
};
use alloy_primitives::{Address, Bytes, U256};

/// Configuration for [`SimulationGate`].
#[derive(Clone, Copy, Debug)]
pub struct SimulationGateConfig {
    /// Gas limit applied when the request doesn't specify one.
    pub default_gas_limit: u64,
}

impl Default for SimulationGateConfig {
    fn default() -> Self {
        Self {
            default_gas_limit: 500_000,
        }
    }
}

/// A `Gate` implementation that validates planned transactions against a
/// mirage fork.
///
/// This gate is cheap to clone — it shares the underlying `MirageFork` via
/// `Arc`. Multiple agents can verify in parallel.
#[derive(Clone, Debug)]
pub struct SimulationGate {
    fork: MirageFork,
    config: SimulationGateConfig,
    name: String,
}

impl SimulationGate {
    /// Constructs a new simulation gate bound to `fork`.
    #[must_use]
    pub fn new(fork: MirageFork) -> Self {
        Self {
            fork,
            config: SimulationGateConfig::default(),
            name: "simulation_gate".to_owned(),
        }
    }

    /// Constructs a new simulation gate with a custom config and name.
    #[must_use]
    pub fn with_config(
        fork: MirageFork,
        config: SimulationGateConfig,
        name: impl Into<String>,
    ) -> Self {
        Self {
            fork,
            config,
            name: name.into(),
        }
    }

    fn parse_request(&self, signal: &Engram) -> Result<ParsedRequest, String> {
        let tx: TransactionRequest = match &signal.body {
            Body::Json(v) => serde_json::from_value(v.clone())
                .map_err(|e| format!("body json does not match TransactionRequest: {e}"))?,
            Body::Text(s) => serde_json::from_str(s)
                .map_err(|e| format!("body text is not valid TransactionRequest json: {e}"))?,
            Body::Bytes(_) => {
                return Err("bytes body is not supported; use text/json".to_owned());
            }
            Body::Empty => {
                return Err("empty body; expected TransactionRequest json".to_owned());
            }
        };
        let from = tx.from.ok_or_else(|| "missing 'from'".to_owned())?;
        let to = tx.to.ok_or_else(|| "missing 'to'".to_owned())?;
        Ok(ParsedRequest {
            from,
            to,
            data: tx.data.unwrap_or_default(),
            value: tx.value.unwrap_or(U256::ZERO),
            gas_limit: tx.gas.unwrap_or(self.config.default_gas_limit),
        })
    }
}

struct ParsedRequest {
    from: Address,
    to: Address,
    data: Bytes,
    value: U256,
    gas_limit: u64,
}

#[async_trait]
impl Gate for SimulationGate {
    async fn verify(&self, signal: &Engram, _ctx: &Context) -> Verdict {
        let started = std::time::Instant::now();
        let parsed = match self.parse_request(signal) {
            Ok(r) => r,
            Err(reason) => {
                let mut v = Verdict::fail(self.name.clone(), reason);
                v.duration_ms = started.elapsed().as_millis() as u64;
                return v;
            }
        };

        // Take a read snapshot of the fork. We perform a read-only simulation —
        // no local block is mined, no dirty state is written.
        let state_lock = self.fork.state();
        let result = {
            let guard = state_lock.read();
            EvmExecutor::call(
                &guard.fork,
                parsed.from,
                parsed.to,
                parsed.data.clone(),
                parsed.value,
                parsed.gas_limit,
            )
        };

        let elapsed = started.elapsed().as_millis() as u64;
        match result {
            Ok(exec) => {
                let output_hex = format!("0x{}", hex_encode(&exec.output));
                let detail = serde_json::json!({
                    "gas_used": exec.gas_used,
                    "output": output_hex,
                    "from": format!("{:?}", parsed.from),
                    "to": format!("{:?}", parsed.to),
                })
                .to_string();
                let mut v = if exec.success {
                    Verdict::pass(self.name.clone())
                } else {
                    Verdict::fail(
                        self.name.clone(),
                        format!("tx reverted (gas_used={})", exec.gas_used),
                    )
                };
                v.detail = Some(detail);
                v.duration_ms = elapsed;
                v
            }
            Err(error) => {
                let reason = match &error {
                    MirageError::Upstream(e) => format!("upstream rpc failed: {e}"),
                    other => format!("simulation error: {other}"),
                };
                let mut v = Verdict::fail(self.name.clone(), reason);
                v.duration_ms = elapsed;
                v
            }
        }
    }

    fn name(&self) -> &str {
        &self.name
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

// The parsed-request struct is an internal helper; serde derives suppressed to
// avoid `unused` lints when tests are off.
#[cfg(test)]
impl ParsedRequest {
    #[allow(dead_code)]
    fn as_debug(&self) -> String {
        format!(
            "from={:?} to={:?} gas={} value={}",
            self.from, self.to, self.gas_limit, self.value
        )
    }
}

// Re-export the Deserialize trait bound shim (no-op: keeps cargo happy about
// the serde import above even when the json path isn't exercised in tests).
#[allow(dead_code)]
fn _unused_deserialize_marker<'de, T: Deserialize<'de>>(_t: T) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        fork::{ForkState, HybridDB, MirageFork},
        provider::UpstreamRpc,
        resources::{MirageMode, Profile, ResourceModel},
    };
    use alloy_primitives::address;
    use roko_core::{Body, Context, Engram, Kind, Provenance};
    use std::{num::NonZeroUsize, sync::Arc, time::Duration};

    fn build_test_fork() -> MirageFork {
        let upstream = Arc::new(UpstreamRpc::mock(1));
        let db = HybridDB::new(upstream, 32, Duration::from_secs(12), NonZeroUsize::MIN, 1);
        let fork = ForkState::new(db, 0, 1);
        MirageFork::new(
            fork,
            ResourceModel::for_profile(Profile::Standard, Duration::from_secs(12)),
            MirageMode::Live,
        )
    }

    fn tx_signal(tx: serde_json::Value) -> Engram {
        Engram::builder(Kind::Transaction)
            .body(Body::Json(tx))
            .provenance(Provenance::agent("alice"))
            .build()
    }

    #[tokio::test]
    async fn simulates_simple_value_transfer() {
        let mirage = build_test_fork();
        let gate = SimulationGate::new(mirage.clone());

        // Seed balances so the transfer is viable.
        {
            let st = mirage.state();
            let mut g = st.write();
            g.fork.db.set_balance(
                address!("0x1000000000000000000000000000000000000001"),
                U256::from(1_000_000_000_000_000_000u64),
            );
        }

        let signal = tx_signal(serde_json::json!({
            "from": "0x1000000000000000000000000000000000000001",
            "to":   "0x2000000000000000000000000000000000000002",
            "value": "0x0",
            "gas":  "0x5208",
            "data": "0x"
        }));
        let verdict = gate.verify(&signal, &Context::now()).await;
        assert!(verdict.passed, "value transfer should succeed: {verdict:?}");
        assert_eq!(verdict.gate, "simulation_gate");
        assert!(verdict.detail.is_some());
    }

    #[tokio::test]
    async fn rejects_signal_without_from() {
        let mirage = build_test_fork();
        let gate = SimulationGate::new(mirage);
        let signal = tx_signal(serde_json::json!({
            "to": "0x2000000000000000000000000000000000000002",
        }));
        let verdict = gate.verify(&signal, &Context::now()).await;
        assert!(!verdict.passed);
        assert!(verdict.reason.contains("missing 'from'"));
    }

    #[tokio::test]
    async fn rejects_empty_body() {
        let mirage = build_test_fork();
        let gate = SimulationGate::new(mirage);
        let signal = Engram::builder(Kind::Transaction)
            .body(Body::Empty)
            .provenance(Provenance::agent("alice"))
            .build();
        let verdict = gate.verify(&signal, &Context::now()).await;
        assert!(!verdict.passed);
        assert!(verdict.reason.contains("empty body"));
    }

    #[tokio::test]
    async fn accepts_text_body_with_json() {
        let mirage = build_test_fork();
        let gate = SimulationGate::new(mirage.clone());
        {
            let st = mirage.state();
            let mut g = st.write();
            g.fork.db.set_balance(
                address!("0x1100000000000000000000000000000000000011"),
                U256::from(1_000_000_000_000_000_000u64),
            );
        }
        let signal = Engram::builder(Kind::Transaction)
            .body(Body::text(
                r#"{"from":"0x1100000000000000000000000000000000000011","to":"0x2200000000000000000000000000000000000022","gas":"0x5208","data":"0x"}"#,
            ))
            .provenance(Provenance::agent("alice"))
            .build();
        let verdict = gate.verify(&signal, &Context::now()).await;
        assert!(verdict.passed);
    }

    #[test]
    fn gate_name_is_configurable() {
        let mirage = build_test_fork();
        let gate = SimulationGate::with_config(
            mirage,
            SimulationGateConfig::default(),
            "my_custom_simulator",
        );
        assert_eq!(gate.name(), "my_custom_simulator");
    }
}
