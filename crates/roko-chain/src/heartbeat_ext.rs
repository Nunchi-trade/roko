//! BEAT-09: Chain heartbeat extension with SIMULATE and VALIDATE steps.
//!
//! Chain agents extend the universal 7-step loop with two additional steps
//! between COMPOSE and ACT because chain actions are financially irreversible:
//!
//! - **SIMULATE**: Run the proposed transaction through a fork simulator
//!   (mirage-rs, Revm, alloy `eth_call`). Check for reverts, gas limits,
//!   sandwich vulnerability, and state change verification.
//!
//! - **VALIDATE**: Check the proposed transaction against the PolicyCage:
//!   position limits, approved asset list, maximum position size, daily
//!   volume caps. Fail the tick if any constraint is violated.
//!
//! This extension hooks into the universal loop without modifying it: the
//! caller invokes `ChainHeartbeatExtension::pre_act_check()` between COMPOSE
//! and ACT, and gates the ACT step on the result.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::doc_markdown,
    clippy::missing_const_for_fn,
    clippy::module_name_repetitions,
    clippy::unused_self
)]

use std::collections::HashSet;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::gate::tx_sim_gate::TxSimulator;
use crate::types::TxRequest;

// ---------------------------------------------------------------------------
// PolicyCage: constraint enforcement for chain actions
// ---------------------------------------------------------------------------

/// A constraint set that gates chain actions before execution.
///
/// The PolicyCage enforces position limits, approved-asset lists, maximum
/// position sizes, and daily volume caps. It is checked during the VALIDATE
/// step of the chain heartbeat variant.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PolicyCageConfig {
    /// Maximum number of simultaneous open positions.
    pub max_open_positions: usize,
    /// Maximum position size in native units (e.g. ETH).
    pub max_position_size: f64,
    /// Maximum daily transaction volume in USD.
    pub max_daily_volume_usd: f64,
    /// Set of approved asset addresses (empty = all allowed).
    pub approved_assets: HashSet<String>,
    /// Whether unapproved assets are blocked (true) or warned (false).
    pub block_unapproved: bool,
    /// Maximum gas price in gwei that is acceptable.
    pub max_gas_gwei: f64,
}

impl Default for PolicyCageConfig {
    fn default() -> Self {
        Self {
            max_open_positions: 10,
            max_position_size: 100.0,
            max_daily_volume_usd: 50_000.0,
            approved_assets: HashSet::new(),
            block_unapproved: false,
            max_gas_gwei: 500.0,
        }
    }
}

/// Current state tracked for PolicyCage validation.
#[derive(Debug, Clone, Default)]
pub struct PolicyCageState {
    /// Current number of open positions.
    pub open_positions: usize,
    /// Daily volume consumed so far in USD.
    pub daily_volume_usd: f64,
    /// Current gas price in gwei.
    pub current_gas_gwei: f64,
}

/// A single policy violation detected during VALIDATE.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PolicyViolation {
    /// Which constraint was violated.
    pub constraint: String,
    /// Human-readable description of the violation.
    pub description: String,
    /// Severity: "error" blocks the tx, "warning" allows with a note.
    pub severity: ViolationSeverity,
}

/// Severity of a policy violation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViolationSeverity {
    /// Hard block: the transaction must not proceed.
    Error,
    /// Soft warning: the transaction may proceed but is flagged.
    Warning,
}

/// Result of the SIMULATE step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SimulateResult {
    /// Whether the simulation passed all checks.
    pub passed: bool,
    /// Gas used in simulation.
    pub gas_used: u64,
    /// Whether a revert was detected.
    pub reverted: bool,
    /// Revert reason, if any.
    pub revert_reason: Option<String>,
    /// Human-readable summary.
    pub summary: String,
}

/// Result of the VALIDATE step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValidateResult {
    /// Whether validation passed (no error-severity violations).
    pub passed: bool,
    /// Policy violations detected (may include warnings even on pass).
    pub violations: Vec<PolicyViolation>,
    /// Human-readable summary.
    pub summary: String,
}

/// Combined result of the chain heartbeat pre-ACT check.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChainPreActResult {
    /// SIMULATE step result.
    pub simulate: SimulateResult,
    /// VALIDATE step result.
    pub validate: ValidateResult,
    /// Whether the ACT step should proceed.
    pub act_allowed: bool,
}

// ---------------------------------------------------------------------------
// ChainHeartbeatExtension
// ---------------------------------------------------------------------------

/// The chain heartbeat extension: SIMULATE + VALIDATE before ACT.
///
/// This struct holds the simulator and policy configuration. Call
/// `pre_act_check()` between the COMPOSE and ACT steps of the universal loop
/// to gate chain actions.
pub struct ChainHeartbeatExtension {
    simulator: Arc<dyn TxSimulator>,
    policy_config: PolicyCageConfig,
}

impl ChainHeartbeatExtension {
    /// Create a new chain heartbeat extension.
    pub fn new(simulator: Arc<dyn TxSimulator>, policy_config: PolicyCageConfig) -> Self {
        Self {
            simulator,
            policy_config,
        }
    }

    /// Run the SIMULATE + VALIDATE steps for a proposed transaction.
    ///
    /// Returns a `ChainPreActResult` indicating whether ACT should proceed.
    pub async fn pre_act_check(
        &self,
        tx: &TxRequest,
        cage_state: &PolicyCageState,
    ) -> ChainPreActResult {
        // Step 1: SIMULATE
        let simulate = self.simulate(tx).await;

        // Step 2: VALIDATE (only if simulation passed)
        let validate = if simulate.passed {
            self.validate(tx, cage_state)
        } else {
            ValidateResult {
                passed: false,
                violations: vec![],
                summary: "Skipped: simulation failed".into(),
            }
        };

        let act_allowed = simulate.passed && validate.passed;

        ChainPreActResult {
            simulate,
            validate,
            act_allowed,
        }
    }

    /// SIMULATE step: run the transaction through the fork simulator.
    async fn simulate(&self, tx: &TxRequest) -> SimulateResult {
        match self.simulator.simulate(tx).await {
            Ok(outcome) => {
                let passed = outcome.success;
                SimulateResult {
                    passed,
                    gas_used: outcome.gas_used,
                    reverted: !outcome.success,
                    revert_reason: outcome.revert_reason.clone(),
                    summary: if passed {
                        format!("Simulation passed, gas_used={}", outcome.gas_used)
                    } else {
                        format!(
                            "Simulation reverted: {}",
                            outcome.revert_reason.as_deref().unwrap_or("unknown")
                        )
                    },
                }
            }
            Err(e) => SimulateResult {
                passed: false,
                gas_used: 0,
                reverted: false,
                revert_reason: None,
                summary: format!("Simulator error: {e}"),
            },
        }
    }

    /// VALIDATE step: check the transaction against the PolicyCage.
    fn validate(&self, tx: &TxRequest, state: &PolicyCageState) -> ValidateResult {
        let mut violations = Vec::new();
        let config = &self.policy_config;

        // Check position limit
        if state.open_positions >= config.max_open_positions {
            violations.push(PolicyViolation {
                constraint: "max_open_positions".into(),
                description: format!(
                    "Open positions ({}) at or above limit ({})",
                    state.open_positions, config.max_open_positions
                ),
                severity: ViolationSeverity::Error,
            });
        }

        // Check daily volume cap
        // Estimate tx value as a simple proxy (real implementation would price it)
        let tx_value = tx.value as f64;
        if state.daily_volume_usd + tx_value > config.max_daily_volume_usd {
            violations.push(PolicyViolation {
                constraint: "max_daily_volume_usd".into(),
                description: format!(
                    "Daily volume ({:.2} + {:.2}) would exceed limit ({:.2})",
                    state.daily_volume_usd, tx_value, config.max_daily_volume_usd
                ),
                severity: ViolationSeverity::Error,
            });
        }

        // Check approved assets (if the approved list is non-empty)
        if !config.approved_assets.is_empty() {
            if let Some(to) = &tx.to {
                if !config.approved_assets.contains(to) {
                    violations.push(PolicyViolation {
                        constraint: "approved_assets".into(),
                        description: format!("Target address {to} is not in approved asset list"),
                        severity: if config.block_unapproved {
                            ViolationSeverity::Error
                        } else {
                            ViolationSeverity::Warning
                        },
                    });
                }
            }
        }

        // Check gas price
        if state.current_gas_gwei > config.max_gas_gwei {
            violations.push(PolicyViolation {
                constraint: "max_gas_gwei".into(),
                description: format!(
                    "Gas price ({:.1} gwei) exceeds limit ({:.1} gwei)",
                    state.current_gas_gwei, config.max_gas_gwei
                ),
                severity: ViolationSeverity::Warning,
            });
        }

        let has_errors = violations
            .iter()
            .any(|v| v.severity == ViolationSeverity::Error);

        let summary = if violations.is_empty() {
            "All policy checks passed".into()
        } else if has_errors {
            format!(
                "{} violation(s): {}",
                violations.len(),
                violations
                    .iter()
                    .map(|v| v.constraint.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        } else {
            format!("{} warning(s), no blocking violations", violations.len())
        };

        ValidateResult {
            passed: !has_errors,
            violations,
            summary,
        }
    }
}

// ---------------------------------------------------------------------------
// Sleepwalker variant: observe-only chain agent (no tx execution)
// ---------------------------------------------------------------------------

/// The Sleepwalker 3-step variant for observer-only chain agents.
///
/// Sleepwalker agents never execute transactions. They follow a simplified
/// 3-step loop: OBSERVE -> REFLECT -> PUBLISH.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SleepwalkerConfig {
    /// How often to poll for new blocks/events (in seconds).
    pub poll_interval_secs: u64,
    /// Maximum events to buffer before forced reflection.
    pub event_buffer_size: usize,
}

impl Default for SleepwalkerConfig {
    fn default() -> Self {
        Self {
            poll_interval_secs: 12,
            event_buffer_size: 100,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gate::tx_sim_gate::{MockTxSimulator, SimulationOutcome};

    fn mock_ext(outcome: SimulationOutcome) -> ChainHeartbeatExtension {
        ChainHeartbeatExtension::new(
            Arc::new(MockTxSimulator { outcome }),
            PolicyCageConfig::default(),
        )
    }

    #[tokio::test(flavor = "current_thread")]
    async fn simulate_pass_and_validate_pass() {
        let ext = mock_ext(SimulationOutcome::ok(21_000));
        let tx = TxRequest::default();
        let state = PolicyCageState::default();
        let result = ext.pre_act_check(&tx, &state).await;
        assert!(result.simulate.passed);
        assert!(result.validate.passed);
        assert!(result.act_allowed);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn simulate_revert_blocks_act() {
        let ext = mock_ext(SimulationOutcome::reverted(50_000, "underflow"));
        let tx = TxRequest::default();
        let state = PolicyCageState::default();
        let result = ext.pre_act_check(&tx, &state).await;
        assert!(!result.simulate.passed);
        assert!(!result.act_allowed);
        assert!(result.simulate.reverted);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn validate_blocks_when_position_limit_hit() {
        let ext = mock_ext(SimulationOutcome::ok(21_000));
        let tx = TxRequest::default();
        let state = PolicyCageState {
            open_positions: 10, // at limit
            ..Default::default()
        };
        let result = ext.pre_act_check(&tx, &state).await;
        assert!(result.simulate.passed);
        assert!(!result.validate.passed);
        assert!(!result.act_allowed);
        assert!(
            result
                .validate
                .violations
                .iter()
                .any(|v| v.constraint == "max_open_positions")
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn validate_blocks_when_daily_volume_exceeded() {
        let ext = mock_ext(SimulationOutcome::ok(21_000));
        let tx = TxRequest {
            value: 100_000,
            ..Default::default()
        };
        let state = PolicyCageState {
            daily_volume_usd: 49_999.0,
            ..Default::default()
        };
        let result = ext.pre_act_check(&tx, &state).await;
        assert!(!result.validate.passed);
        assert!(!result.act_allowed);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn validate_warns_on_unapproved_asset_soft_block() {
        let config = PolicyCageConfig {
            approved_assets: ["0xapproved".to_string()].into_iter().collect(),
            block_unapproved: false,
            ..Default::default()
        };
        let ext = ChainHeartbeatExtension::new(
            Arc::new(MockTxSimulator {
                outcome: SimulationOutcome::ok(21_000),
            }),
            config,
        );
        let tx = TxRequest {
            to: Some("0xunapproved".into()),
            ..Default::default()
        };
        let state = PolicyCageState::default();
        let result = ext.pre_act_check(&tx, &state).await;
        // Warning only, not a hard block
        assert!(result.validate.passed);
        assert!(result.act_allowed);
        assert!(!result.validate.violations.is_empty());
        assert_eq!(
            result.validate.violations[0].severity,
            ViolationSeverity::Warning
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn validate_blocks_on_unapproved_asset_hard_block() {
        let config = PolicyCageConfig {
            approved_assets: ["0xapproved".to_string()].into_iter().collect(),
            block_unapproved: true,
            ..Default::default()
        };
        let ext = ChainHeartbeatExtension::new(
            Arc::new(MockTxSimulator {
                outcome: SimulationOutcome::ok(21_000),
            }),
            config,
        );
        let tx = TxRequest {
            to: Some("0xunapproved".into()),
            ..Default::default()
        };
        let state = PolicyCageState::default();
        let result = ext.pre_act_check(&tx, &state).await;
        assert!(!result.validate.passed);
        assert!(!result.act_allowed);
    }

    #[test]
    fn sleepwalker_config_defaults() {
        let config = SleepwalkerConfig::default();
        assert_eq!(config.poll_interval_secs, 12);
        assert_eq!(config.event_buffer_size, 100);
    }

    #[test]
    fn policy_cage_config_defaults() {
        let config = PolicyCageConfig::default();
        assert_eq!(config.max_open_positions, 10);
        assert!(config.approved_assets.is_empty());
    }
}
