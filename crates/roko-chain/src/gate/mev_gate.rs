//! [`MevGate`] -- pre-flight MEV detection gate for chain-domain agents.
//!
//! Detects common MEV attack patterns (sandwich, front-running, back-running,
//! JIT liquidity, cyclic arbitrage) by analyzing a bundle of transactions
//! relative to a pending victim transaction. This gate is **standalone** --
//! it is not part of the 7-rung pipeline but is registered as an optional
//! pre-flight check for agents with `domain = "chain"`.
//!
//! # Detection algorithms
//!
//! The [`MevDetector`] runs five detection passes in order:
//!
//! 1. **Sandwich**: three-tx pattern on the same pool (frontrun → victim → backrun)
//!    by the same attacker address, with the attacker profiting from the price impact.
//! 2. **Front-running**: single tx submitted ahead of the victim tx targeting the
//!    same pool/function selector, with a higher gas price.
//! 3. **Back-running**: single tx submitted after the victim tx, exploiting the
//!    state change caused by the victim (e.g., liquidation, arbitrage).
//! 4. **JIT liquidity**: liquidity added just before the victim's swap and removed
//!    immediately after, capturing fees without exposure.
//! 5. **Cyclic arbitrage**: a cycle of swaps across multiple pools that returns
//!    to the starting token with a profit (no net position change).
//!
//! Each detected pattern is returned as a [`MevAlert`] with severity, profit
//! estimate, and the offending transaction hashes.
//!
//! # Verify integration
//!
//! `MevGate` implements [`Verify`](roko_core::Verify). It reads the signal body as
//! a JSON object containing:
//!
//! - `victim_tx`: the agent's planned transaction (same format as `TxRequest`)
//! - `mempool_txs`: an array of nearby pending transactions to scan
//!
//! If any high-severity MEV pattern is detected, the gate fails the signal.

use async_trait::async_trait;
use roko_core::{Body, Context, Engram, traits::Verify, verdict::Verdict};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::time::Instant;

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

// ─── Core types ──────────────────────────────────────────────────────────

/// A simplified mempool transaction for MEV analysis.
///
/// This is intentionally narrow -- it carries only the fields needed for
/// pattern detection, not the full Ethereum transaction. Real backends
/// populate these from pending-tx subscription data; mocks and tests
/// construct them directly.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MempoolTx {
    /// Transaction hash (hex, `0x`-prefixed).
    pub hash: String,
    /// Sender address (hex, `0x`-prefixed, lowercased).
    pub from: String,
    /// Recipient address (hex, `0x`-prefixed, lowercased). `None` for contract creation.
    pub to: Option<String>,
    /// Value in wei.
    #[serde(default)]
    pub value: u128,
    /// First 4 bytes of calldata (function selector) as hex, e.g. `"0x38ed1739"`.
    #[serde(default)]
    pub selector: Option<String>,
    /// Gas price or effective gas price (wei).
    #[serde(default)]
    pub gas_price: u128,
    /// Pool or contract address this tx interacts with (extracted from calldata).
    #[serde(default)]
    pub target_pool: Option<String>,
    /// Position in the pending tx ordering (lower = earlier).
    #[serde(default)]
    pub position: u64,
}

/// The victim transaction and its mempool context, submitted for MEV analysis.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MevAnalysisInput {
    /// The agent's planned transaction.
    pub victim_tx: MempoolTx,
    /// Nearby pending transactions from the mempool.
    #[serde(default)]
    pub mempool_txs: Vec<MempoolTx>,
}

/// A three-transaction sandwich bundle.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SandwichBundle {
    /// Attacker's address.
    pub attacker: String,
    /// Frontrun transaction hash.
    pub frontrun_tx: String,
    /// Victim transaction hash.
    pub victim_tx: String,
    /// Backrun transaction hash.
    pub backrun_tx: String,
    /// Estimated profit in wei (backrun value - frontrun cost, approximate).
    pub estimated_profit_wei: u128,
    /// The pool or contract targeted.
    pub target_pool: String,
}

/// Classification of detected MEV patterns.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MevPattern {
    /// Three-tx sandwich on the same pool.
    Sandwich,
    /// Single tx ahead of victim targeting the same pool with higher gas.
    FrontRun,
    /// Single tx after victim exploiting state change.
    BackRun,
    /// Liquidity added before and removed after victim's swap.
    JitLiquidity,
    /// Cyclic swap path returning to starting token.
    CyclicArbitrage,
}

impl std::fmt::Display for MevPattern {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sandwich => write!(f, "sandwich"),
            Self::FrontRun => write!(f, "front-run"),
            Self::BackRun => write!(f, "back-run"),
            Self::JitLiquidity => write!(f, "JIT-liquidity"),
            Self::CyclicArbitrage => write!(f, "cyclic-arb"),
        }
    }
}

/// Severity of a detected MEV pattern.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum MevSeverity {
    /// Informational -- pattern detected but low confidence or low impact.
    Info,
    /// Warning -- pattern likely, moderate impact.
    Warning,
    /// Critical -- high-confidence sandwich or front-run with significant profit.
    Critical,
}

/// A single MEV detection alert.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MevAlert {
    /// The type of MEV pattern detected.
    pub pattern: MevPattern,
    /// Severity of the detection.
    pub severity: MevSeverity,
    /// Human-readable description of what was found.
    pub description: String,
    /// Transaction hashes involved (attacker txs).
    pub involved_txs: Vec<String>,
    /// Estimated attacker profit in wei (0 if unknown).
    pub estimated_profit_wei: u128,
    /// The sandwich bundle details, if this is a sandwich detection.
    pub sandwich: Option<SandwichBundle>,
}

// ─── Detector ────────────────────────────────────────────────────────────

/// Configuration for [`MevDetector`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MevDetectorConfig {
    /// Minimum estimated profit (wei) to flag as critical severity.
    /// Below this threshold, detections are reported as warnings.
    #[serde(default = "default_min_profit_threshold_wei")]
    pub min_profit_threshold_wei: u128,
    /// Known bot addresses (lowercased hex). Transactions from these addresses
    /// receive elevated severity.
    #[serde(default)]
    pub known_bots: HashMap<String, String>,
    /// Whether to check for sandwich patterns.
    #[serde(default = "default_true")]
    pub detect_sandwich: bool,
    /// Whether to check for front-running patterns.
    #[serde(default = "default_true")]
    pub detect_frontrun: bool,
    /// Whether to check for back-running patterns.
    #[serde(default = "default_true")]
    pub detect_backrun: bool,
    /// Whether to check for JIT liquidity patterns.
    #[serde(default = "default_true")]
    pub detect_jit: bool,
    /// Whether to check for cyclic arbitrage patterns.
    #[serde(default = "default_true")]
    pub detect_cyclic: bool,
}

fn default_min_profit_threshold_wei() -> u128 {
    // 0.01 ETH in wei
    10_000_000_000_000_000
}

fn default_true() -> bool {
    true
}

impl Default for MevDetectorConfig {
    fn default() -> Self {
        Self {
            min_profit_threshold_wei: default_min_profit_threshold_wei(),
            known_bots: HashMap::new(),
            detect_sandwich: true,
            detect_frontrun: true,
            detect_backrun: true,
            detect_jit: true,
            detect_cyclic: true,
        }
    }
}

/// MEV detector that analyzes a victim transaction against mempool context.
///
/// Runs up to five detection algorithms in sequence, accumulating alerts.
/// The detector is stateless -- each `detect()` call is independent.
pub struct MevDetector {
    config: MevDetectorConfig,
}

impl MevDetector {
    /// Create a new detector with default configuration.
    #[must_use]
    pub fn new() -> Self {
        Self {
            config: MevDetectorConfig::default(),
        }
    }

    /// Create a detector with custom configuration.
    #[must_use]
    pub fn with_config(config: MevDetectorConfig) -> Self {
        Self { config }
    }

    /// Run all enabled detection algorithms on the given input.
    #[must_use]
    pub fn detect(&self, input: &MevAnalysisInput) -> Vec<MevAlert> {
        let mut alerts = Vec::new();

        if self.config.detect_sandwich {
            alerts.extend(self.detect_sandwich(input));
        }
        if self.config.detect_frontrun {
            alerts.extend(self.detect_frontrun(input));
        }
        if self.config.detect_backrun {
            alerts.extend(self.detect_backrun(input));
        }
        if self.config.detect_jit {
            alerts.extend(self.detect_jit_liquidity(input));
        }
        if self.config.detect_cyclic {
            alerts.extend(self.detect_cyclic_arbitrage(input));
        }

        alerts
    }

    /// Detect sandwich attacks: frontrun → victim → backrun by same attacker on same pool.
    fn detect_sandwich(&self, input: &MevAnalysisInput) -> Vec<MevAlert> {
        let mut alerts = Vec::new();
        let victim = &input.victim_tx;

        let Some(victim_pool) = victim.target_pool.as_deref() else {
            return alerts;
        };

        // Group mempool txs by sender that target the same pool.
        let mut by_sender: HashMap<&str, Vec<&MempoolTx>> = HashMap::new();
        for tx in &input.mempool_txs {
            if tx.target_pool.as_deref() == Some(victim_pool) && tx.from != victim.from {
                by_sender.entry(tx.from.as_str()).or_default().push(tx);
            }
        }

        for (attacker, txs) in &by_sender {
            // Need at least 2 txs from the same sender: one before and one after victim.
            let before: Vec<&&MempoolTx> = txs
                .iter()
                .filter(|t| t.position < victim.position)
                .collect();
            let after: Vec<&&MempoolTx> = txs
                .iter()
                .filter(|t| t.position > victim.position)
                .collect();

            if !before.is_empty() && !after.is_empty() {
                let frontrun = before[0];
                let backrun = after[0];

                // Rough profit estimate: backrun value - frontrun value (very approximate).
                let profit = backrun.value.saturating_sub(frontrun.value);

                let severity = self.classify_severity(attacker, profit);

                let bundle = SandwichBundle {
                    attacker: attacker.to_string(),
                    frontrun_tx: frontrun.hash.clone(),
                    victim_tx: victim.hash.clone(),
                    backrun_tx: backrun.hash.clone(),
                    estimated_profit_wei: profit,
                    target_pool: victim_pool.to_string(),
                };

                alerts.push(MevAlert {
                    pattern: MevPattern::Sandwich,
                    severity,
                    description: format!(
                        "sandwich attack on pool {} by {}: frontrun {} → victim {} → backrun {}",
                        victim_pool, attacker, frontrun.hash, victim.hash, backrun.hash
                    ),
                    involved_txs: vec![frontrun.hash.clone(), backrun.hash.clone()],
                    estimated_profit_wei: profit,
                    sandwich: Some(bundle),
                });
            }
        }

        alerts
    }

    /// Detect front-running: a tx targeting the same pool submitted ahead with higher gas.
    fn detect_frontrun(&self, input: &MevAnalysisInput) -> Vec<MevAlert> {
        let mut alerts = Vec::new();
        let victim = &input.victim_tx;

        let Some(victim_pool) = victim.target_pool.as_deref() else {
            return alerts;
        };

        for tx in &input.mempool_txs {
            if tx.from == victim.from {
                continue;
            }
            let same_pool = tx.target_pool.as_deref() == Some(victim_pool);
            let same_selector = tx.selector.is_some() && tx.selector == victim.selector;
            let ahead = tx.position < victim.position;
            let higher_gas = tx.gas_price > victim.gas_price;

            if same_pool && same_selector && ahead && higher_gas {
                let severity = self.classify_severity(&tx.from, 0);
                alerts.push(MevAlert {
                    pattern: MevPattern::FrontRun,
                    severity,
                    description: format!(
                        "front-run on pool {} by {}: tx {} (gas {}) ahead of victim (gas {})",
                        victim_pool, tx.from, tx.hash, tx.gas_price, victim.gas_price
                    ),
                    involved_txs: vec![tx.hash.clone()],
                    estimated_profit_wei: 0,
                    sandwich: None,
                });
            }
        }

        alerts
    }

    /// Detect back-running: a tx exploiting state change after victim.
    fn detect_backrun(&self, input: &MevAnalysisInput) -> Vec<MevAlert> {
        let mut alerts = Vec::new();
        let victim = &input.victim_tx;

        let Some(victim_pool) = victim.target_pool.as_deref() else {
            return alerts;
        };

        for tx in &input.mempool_txs {
            if tx.from == victim.from {
                continue;
            }
            let same_pool = tx.target_pool.as_deref() == Some(victim_pool);
            let after = tx.position > victim.position;
            let different_selector = tx.selector != victim.selector;
            let is_known_bot = self.config.known_bots.contains_key(&tx.from);

            // Back-run: same pool, right after victim, different function (arbitrage),
            // typically from a known bot.
            if same_pool && after && (different_selector || is_known_bot) {
                let severity = if is_known_bot {
                    MevSeverity::Warning
                } else {
                    MevSeverity::Info
                };
                alerts.push(MevAlert {
                    pattern: MevPattern::BackRun,
                    severity,
                    description: format!(
                        "potential back-run on pool {} by {}: tx {} after victim",
                        victim_pool, tx.from, tx.hash
                    ),
                    involved_txs: vec![tx.hash.clone()],
                    estimated_profit_wei: 0,
                    sandwich: None,
                });
            }
        }

        alerts
    }

    /// Detect JIT liquidity: add liquidity before victim swap, remove after.
    ///
    /// Heuristic: look for add-liquidity selector before and remove-liquidity
    /// selector after the victim, from the same address, on the same pool.
    fn detect_jit_liquidity(&self, input: &MevAnalysisInput) -> Vec<MevAlert> {
        let mut alerts = Vec::new();
        let victim = &input.victim_tx;

        let Some(victim_pool) = victim.target_pool.as_deref() else {
            return alerts;
        };

        // Common Uniswap V3 selectors (simplified).
        const ADD_LIQ_SELECTORS: &[&str] = &["0xe8e33700", "0x88316456", "0x219f5d17"];
        const REMOVE_LIQ_SELECTORS: &[&str] = &["0xbaa2abde", "0x0c49ccbe", "0xfc6f7865"];

        let mut adds_before: HashMap<&str, Vec<&MempoolTx>> = HashMap::new();
        let mut removes_after: HashMap<&str, Vec<&MempoolTx>> = HashMap::new();

        for tx in &input.mempool_txs {
            if tx.target_pool.as_deref() != Some(victim_pool) {
                continue;
            }
            let sel = tx.selector.as_deref().unwrap_or("");
            if tx.position < victim.position && ADD_LIQ_SELECTORS.contains(&sel) {
                adds_before.entry(tx.from.as_str()).or_default().push(tx);
            }
            if tx.position > victim.position && REMOVE_LIQ_SELECTORS.contains(&sel) {
                removes_after.entry(tx.from.as_str()).or_default().push(tx);
            }
        }

        for (provider, add_txs) in &adds_before {
            if let Some(remove_txs) = removes_after.get(provider) {
                let severity = self.classify_severity(provider, 0);
                let mut involved = Vec::new();
                for t in add_txs {
                    involved.push(t.hash.clone());
                }
                for t in remove_txs {
                    involved.push(t.hash.clone());
                }
                alerts.push(MevAlert {
                    pattern: MevPattern::JitLiquidity,
                    severity,
                    description: format!(
                        "JIT liquidity on pool {} by {}: add before victim, remove after",
                        victim_pool, provider
                    ),
                    involved_txs: involved,
                    estimated_profit_wei: 0,
                    sandwich: None,
                });
            }
        }

        alerts
    }

    /// Detect cyclic arbitrage: a sequence of swaps that returns to the starting token.
    ///
    /// Heuristic: from a single sender, if we see swaps across 2+ pools forming
    /// a cycle (token A → B → ... → A), flag it.
    fn detect_cyclic_arbitrage(&self, input: &MevAnalysisInput) -> Vec<MevAlert> {
        let mut alerts = Vec::new();

        // Group by sender, look for multiple pool interactions.
        let mut by_sender: HashMap<&str, HashSet<&str>> = HashMap::new();
        for tx in &input.mempool_txs {
            if let Some(pool) = tx.target_pool.as_deref() {
                by_sender.entry(tx.from.as_str()).or_default().insert(pool);
            }
        }

        for (sender, pools) in &by_sender {
            if pools.len() >= 2 {
                let is_known_bot = self.config.known_bots.contains_key(*sender);
                // Multiple pool interactions from same sender suggest cyclic arb.
                let severity = if is_known_bot || pools.len() >= 3 {
                    MevSeverity::Warning
                } else {
                    MevSeverity::Info
                };

                let involved: Vec<String> = input
                    .mempool_txs
                    .iter()
                    .filter(|t| t.from.as_str() == *sender && t.target_pool.is_some())
                    .map(|t| t.hash.clone())
                    .collect();

                if involved.len() >= 2 {
                    alerts.push(MevAlert {
                        pattern: MevPattern::CyclicArbitrage,
                        severity,
                        description: format!(
                            "potential cyclic arbitrage by {} across {} pools",
                            sender,
                            pools.len()
                        ),
                        involved_txs: involved,
                        estimated_profit_wei: 0,
                        sandwich: None,
                    });
                }
            }
        }

        alerts
    }

    /// Classify severity based on attacker address and profit estimate.
    fn classify_severity(&self, address: &str, profit_wei: u128) -> MevSeverity {
        if self.config.known_bots.contains_key(address) {
            return MevSeverity::Critical;
        }
        if profit_wei >= self.config.min_profit_threshold_wei {
            return MevSeverity::Critical;
        }
        if profit_wei > 0 {
            return MevSeverity::Warning;
        }
        MevSeverity::Info
    }

    /// Returns `true` if any alert has critical severity.
    #[must_use]
    pub fn has_critical(alerts: &[MevAlert]) -> bool {
        alerts.iter().any(|a| a.severity == MevSeverity::Critical)
    }

    /// Returns `true` if any alert has warning or critical severity.
    #[must_use]
    pub fn has_warnings(alerts: &[MevAlert]) -> bool {
        alerts.iter().any(|a| a.severity >= MevSeverity::Warning)
    }
}

impl Default for MevDetector {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Verify ────────────────────────────────────────────────────────────────

/// A [`Verify`] that runs MEV detection on a planned transaction before signing.
///
/// The gate reads the signal body as a JSON-encoded [`MevAnalysisInput`]
/// containing the victim transaction and mempool context. If any critical
/// MEV pattern is detected, the gate fails. Warnings are included in the
/// verdict detail but do not cause a failure.
///
/// This gate is standalone (not part of the 7-rung pipeline) and is
/// intended for chain-domain agents only.
pub struct MevGate {
    detector: MevDetector,
    name: String,
    /// When `true`, even warning-level alerts cause the gate to fail.
    fail_on_warning: bool,
}

impl MevGate {
    /// Create a new MEV gate with default configuration.
    #[must_use]
    pub fn new() -> Self {
        Self {
            detector: MevDetector::new(),
            name: "mev_gate".to_string(),
            fail_on_warning: false,
        }
    }

    /// Create a MEV gate with custom detector configuration.
    #[must_use]
    pub fn with_config(config: MevDetectorConfig) -> Self {
        Self {
            detector: MevDetector::with_config(config),
            name: "mev_gate".to_string(),
            fail_on_warning: false,
        }
    }

    /// Set whether warnings (not just critical) should fail the gate.
    #[must_use]
    pub fn fail_on_warning(mut self, yes: bool) -> Self {
        self.fail_on_warning = yes;
        self
    }

    /// Override the gate name.
    #[must_use]
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// Parse the signal body into [`MevAnalysisInput`].
    fn parse_input(signal: &Engram) -> Result<MevAnalysisInput, String> {
        match &signal.body {
            Body::Json(v) => serde_json::from_value(v.clone())
                .map_err(|e| format!("body json does not match MevAnalysisInput: {e}")),
            Body::Text(t) => serde_json::from_str(t)
                .map_err(|e| format!("body text is not valid MevAnalysisInput JSON: {e}")),
            Body::Empty => Err("empty body; expected MevAnalysisInput JSON".to_string()),
            Body::Bytes(_) => Err("bytes body is not supported; expected JSON".to_string()),
        }
    }
}

impl Default for MevGate {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Verify for MevGate {
    async fn verify(&self, signal: &Engram, _ctx: &Context) -> Verdict {
        let started = Instant::now();

        let input = match Self::parse_input(signal) {
            Ok(i) => i,
            Err(reason) => {
                return Verdict::fail(self.name.clone(), reason).with_duration(elapsed_ms(started));
            }
        };

        let alerts = self.detector.detect(&input);

        if alerts.is_empty() {
            return Verdict::pass(&self.name)
                .with_detail("no MEV patterns detected".to_string())
                .with_duration(elapsed_ms(started));
        }

        // Build detail summary.
        let detail = alerts
            .iter()
            .map(|a| format!("[{:?}] {}: {}", a.severity, a.pattern, a.description))
            .collect::<Vec<_>>()
            .join("; ");

        let should_fail = MevDetector::has_critical(&alerts)
            || (self.fail_on_warning && MevDetector::has_warnings(&alerts));

        if should_fail {
            let critical_count = alerts
                .iter()
                .filter(|a| a.severity == MevSeverity::Critical)
                .count();
            let warn_count = alerts
                .iter()
                .filter(|a| a.severity == MevSeverity::Warning)
                .count();
            Verdict::fail(
                self.name.clone(),
                format!(
                    "MEV detected: {} critical, {} warning alerts",
                    critical_count, warn_count
                ),
            )
            .with_detail(detail)
            .with_duration(elapsed_ms(started))
        } else {
            Verdict::pass(&self.name)
                .with_detail(format!("MEV warnings (non-blocking): {detail}"))
                .with_duration(elapsed_ms(started))
        }
    }

    fn name(&self) -> &str {
        &self.name
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use roko_core::{Body, Context, Engram, Kind, Provenance};

    fn victim_tx() -> MempoolTx {
        MempoolTx {
            hash: "0xvictim".to_string(),
            from: "0xuser".to_string(),
            to: Some("0xrouter".to_string()),
            value: 1_000_000,
            selector: Some("0x38ed1739".to_string()),
            gas_price: 50_000_000_000,
            target_pool: Some("0xpool_ab".to_string()),
            position: 5,
        }
    }

    fn signal_from_input(input: &MevAnalysisInput) -> Engram {
        Engram::builder(Kind::Transaction)
            .body(Body::Json(serde_json::to_value(input).unwrap()))
            .provenance(Provenance::agent("chain-agent"))
            .build()
    }

    #[test]
    fn no_mempool_no_alerts() {
        let det = MevDetector::new();
        let input = MevAnalysisInput {
            victim_tx: victim_tx(),
            mempool_txs: vec![],
        };
        let alerts = det.detect(&input);
        assert!(alerts.is_empty());
    }

    #[test]
    fn sandwich_detected() {
        let det = MevDetector::new();
        let input = MevAnalysisInput {
            victim_tx: victim_tx(),
            mempool_txs: vec![
                MempoolTx {
                    hash: "0xfront".to_string(),
                    from: "0xbot".to_string(),
                    to: Some("0xrouter".to_string()),
                    value: 500_000,
                    selector: Some("0x38ed1739".to_string()),
                    gas_price: 100_000_000_000,
                    target_pool: Some("0xpool_ab".to_string()),
                    position: 4,
                },
                MempoolTx {
                    hash: "0xback".to_string(),
                    from: "0xbot".to_string(),
                    to: Some("0xrouter".to_string()),
                    value: 1_500_000,
                    selector: Some("0x38ed1739".to_string()),
                    gas_price: 100_000_000_000,
                    target_pool: Some("0xpool_ab".to_string()),
                    position: 6,
                },
            ],
        };
        let alerts = det.detect(&input);
        let sandwich_alerts: Vec<_> = alerts
            .iter()
            .filter(|a| a.pattern == MevPattern::Sandwich)
            .collect();
        assert_eq!(sandwich_alerts.len(), 1);
        let a = &sandwich_alerts[0];
        assert!(a.sandwich.is_some());
        let bundle = a.sandwich.as_ref().unwrap();
        assert_eq!(bundle.attacker, "0xbot");
        assert_eq!(bundle.frontrun_tx, "0xfront");
        assert_eq!(bundle.backrun_tx, "0xback");
    }

    #[test]
    fn frontrun_detected() {
        let det = MevDetector::new();
        let input = MevAnalysisInput {
            victim_tx: victim_tx(),
            mempool_txs: vec![MempoolTx {
                hash: "0xfr".to_string(),
                from: "0xbot".to_string(),
                to: Some("0xrouter".to_string()),
                value: 0,
                selector: Some("0x38ed1739".to_string()),
                gas_price: 100_000_000_000,
                target_pool: Some("0xpool_ab".to_string()),
                position: 3,
            }],
        };
        let alerts = det.detect(&input);
        assert!(
            alerts.iter().any(|a| a.pattern == MevPattern::FrontRun),
            "expected FrontRun: {alerts:?}"
        );
    }

    #[test]
    fn known_bot_elevates_severity() {
        let mut bots = HashMap::new();
        bots.insert("0xbot".to_string(), "jaredfromsubway".to_string());
        let det = MevDetector::with_config(MevDetectorConfig {
            known_bots: bots,
            ..Default::default()
        });
        let input = MevAnalysisInput {
            victim_tx: victim_tx(),
            mempool_txs: vec![MempoolTx {
                hash: "0xfr".to_string(),
                from: "0xbot".to_string(),
                to: Some("0xrouter".to_string()),
                value: 0,
                selector: Some("0x38ed1739".to_string()),
                gas_price: 100_000_000_000,
                target_pool: Some("0xpool_ab".to_string()),
                position: 3,
            }],
        };
        let alerts = det.detect(&input);
        assert!(
            alerts.iter().any(|a| a.severity == MevSeverity::Critical),
            "known bot should be critical"
        );
    }

    #[test]
    fn cyclic_arb_detected() {
        let det = MevDetector::new();
        let input = MevAnalysisInput {
            victim_tx: victim_tx(),
            mempool_txs: vec![
                MempoolTx {
                    hash: "0xarb1".to_string(),
                    from: "0xarber".to_string(),
                    to: Some("0xrouter".to_string()),
                    value: 0,
                    selector: Some("0xswap1".to_string()),
                    gas_price: 50_000_000_000,
                    target_pool: Some("0xpool_ab".to_string()),
                    position: 7,
                },
                MempoolTx {
                    hash: "0xarb2".to_string(),
                    from: "0xarber".to_string(),
                    to: Some("0xrouter".to_string()),
                    value: 0,
                    selector: Some("0xswap2".to_string()),
                    gas_price: 50_000_000_000,
                    target_pool: Some("0xpool_bc".to_string()),
                    position: 8,
                },
            ],
        };
        let alerts = det.detect(&input);
        assert!(
            alerts
                .iter()
                .any(|a| a.pattern == MevPattern::CyclicArbitrage),
            "expected cyclic arb: {alerts:?}"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn gate_passes_on_no_mev() {
        let gate = MevGate::new();
        let input = MevAnalysisInput {
            victim_tx: victim_tx(),
            mempool_txs: vec![],
        };
        let signal = signal_from_input(&input);
        let verdict = gate.verify(&signal, &Context::now()).await;
        assert!(verdict.passed, "verdict: {verdict:?}");
        assert_eq!(verdict.gate, "mev_gate");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn gate_fails_on_critical_sandwich() {
        let mut bots = HashMap::new();
        bots.insert("0xbot".to_string(), "known-mev-bot".to_string());
        let gate = MevGate::with_config(MevDetectorConfig {
            known_bots: bots,
            ..Default::default()
        });
        let input = MevAnalysisInput {
            victim_tx: victim_tx(),
            mempool_txs: vec![
                MempoolTx {
                    hash: "0xfront".to_string(),
                    from: "0xbot".to_string(),
                    to: Some("0xrouter".to_string()),
                    value: 500_000,
                    selector: Some("0x38ed1739".to_string()),
                    gas_price: 100_000_000_000,
                    target_pool: Some("0xpool_ab".to_string()),
                    position: 4,
                },
                MempoolTx {
                    hash: "0xback".to_string(),
                    from: "0xbot".to_string(),
                    to: Some("0xrouter".to_string()),
                    value: 1_500_000,
                    selector: Some("0x38ed1739".to_string()),
                    gas_price: 100_000_000_000,
                    target_pool: Some("0xpool_ab".to_string()),
                    position: 6,
                },
            ],
        };
        let signal = signal_from_input(&input);
        let verdict = gate.verify(&signal, &Context::now()).await;
        assert!(!verdict.passed, "should fail on critical MEV: {verdict:?}");
        assert!(verdict.reason.contains("MEV detected"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn gate_passes_on_info_only() {
        // Back-run from unknown sender → Info severity → gate passes.
        let gate = MevGate::new();
        let input = MevAnalysisInput {
            victim_tx: victim_tx(),
            mempool_txs: vec![MempoolTx {
                hash: "0xbr".to_string(),
                from: "0xunknown".to_string(),
                to: Some("0xrouter".to_string()),
                value: 0,
                selector: Some("0xdifferent".to_string()),
                gas_price: 50_000_000_000,
                target_pool: Some("0xpool_ab".to_string()),
                position: 6,
            }],
        };
        let signal = signal_from_input(&input);
        let verdict = gate.verify(&signal, &Context::now()).await;
        assert!(verdict.passed, "info-only should pass: {verdict:?}");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn gate_fail_on_warning_when_configured() {
        let gate = MevGate::new().fail_on_warning(true);
        let mut bots = HashMap::new();
        bots.insert("0xsusbot".to_string(), "sus".to_string());
        // A back-run from a known bot → Warning → should fail when fail_on_warning.
        let gate = MevGate::with_config(MevDetectorConfig {
            known_bots: bots,
            ..Default::default()
        })
        .fail_on_warning(true);
        let input = MevAnalysisInput {
            victim_tx: victim_tx(),
            mempool_txs: vec![MempoolTx {
                hash: "0xbr".to_string(),
                from: "0xsusbot".to_string(),
                to: Some("0xrouter".to_string()),
                value: 0,
                selector: Some("0xdifferent".to_string()),
                gas_price: 50_000_000_000,
                target_pool: Some("0xpool_ab".to_string()),
                position: 6,
            }],
        };
        let signal = signal_from_input(&input);
        let verdict = gate.verify(&signal, &Context::now()).await;
        assert!(!verdict.passed, "should fail on warning: {verdict:?}");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn gate_fails_on_bad_body() {
        let gate = MevGate::new();
        let signal = Engram::builder(Kind::Transaction)
            .body(Body::Empty)
            .provenance(Provenance::agent("agent"))
            .build();
        let verdict = gate.verify(&signal, &Context::now()).await;
        assert!(!verdict.passed);
        assert!(verdict.reason.contains("empty body"));
    }

    #[test]
    fn detector_default() {
        let det = MevDetector::default();
        let input = MevAnalysisInput {
            victim_tx: victim_tx(),
            mempool_txs: vec![],
        };
        assert!(det.detect(&input).is_empty());
    }

    #[test]
    fn gate_name() {
        let gate = MevGate::new().with_name("custom_mev");
        assert_eq!(gate.name(), "custom_mev");
    }

    #[test]
    fn has_critical_and_warnings() {
        let alerts = vec![MevAlert {
            pattern: MevPattern::FrontRun,
            severity: MevSeverity::Info,
            description: "info".to_string(),
            involved_txs: vec![],
            estimated_profit_wei: 0,
            sandwich: None,
        }];
        assert!(!MevDetector::has_critical(&alerts));
        assert!(!MevDetector::has_warnings(&alerts));

        let alerts = vec![MevAlert {
            pattern: MevPattern::Sandwich,
            severity: MevSeverity::Warning,
            description: "warn".to_string(),
            involved_txs: vec![],
            estimated_profit_wei: 0,
            sandwich: None,
        }];
        assert!(!MevDetector::has_critical(&alerts));
        assert!(MevDetector::has_warnings(&alerts));
    }
}
