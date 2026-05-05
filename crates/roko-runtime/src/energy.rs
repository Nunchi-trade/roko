//! STATUS: NOT WIRED -- built but no non-test runtime caller.
//!
//! Cognitive energy model -- metabolic costs for cognitive operations.
//!
//! Agents have a finite energy pool that is consumed by operations (LLM calls,
//! tool invocations, gate runs) and replenished over time. When energy is low,
//! the system throttles expensive operations and favors cheaper alternatives.
//!
//! # Architecture
//!
//! ```text
//! EnergyPool ──────── available cognitive energy (USD-denominated)
//! CognitiveMetabolism  energy consumption rates per operation type
//! EnergyLedger ─────── tracks spend/replenish events for accounting
//! ```

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// The type of cognitive operation consuming energy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum OperationKind {
    /// An LLM inference call.
    LlmCall,
    /// A tool invocation (file read, shell command, etc.).
    ToolCall,
    /// A gate verification run (compile, test, clippy).
    GateRun,
    /// Context assembly / prompt composition.
    ContextAssembly,
    /// Knowledge store query.
    KnowledgeQuery,
    /// Research / web search.
    Research,
    /// Plan generation or re-planning.
    Planning,
    /// Checkpoint / snapshot write.
    Checkpoint,
}

impl OperationKind {
    /// Default cost in USD for this operation type.
    #[must_use]
    pub const fn default_cost(&self) -> f64 {
        match self {
            Self::LlmCall => 0.01,
            Self::ToolCall => 0.001,
            Self::GateRun => 0.005,
            Self::ContextAssembly => 0.002,
            Self::KnowledgeQuery => 0.001,
            Self::Research => 0.02,
            Self::Planning => 0.015,
            Self::Checkpoint => 0.0005,
        }
    }

    /// Human-readable label.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::LlmCall => "llm_call",
            Self::ToolCall => "tool_call",
            Self::GateRun => "gate_run",
            Self::ContextAssembly => "context_assembly",
            Self::KnowledgeQuery => "knowledge_query",
            Self::Research => "research",
            Self::Planning => "planning",
            Self::Checkpoint => "checkpoint",
        }
    }
}

/// Metabolism rates governing energy consumption per operation type.
///
/// Rates are multipliers applied to the base cost of each operation kind.
/// A rate of 1.0 means default cost; 2.0 means double cost (e.g., premium model).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CognitiveMetabolism {
    /// Per-operation-kind cost multipliers.
    pub rates: HashMap<OperationKind, f64>,
    /// Global multiplier applied on top of per-kind rates.
    /// Use > 1.0 for high-performance mode, < 1.0 for economy mode.
    pub global_multiplier: f64,
}

impl CognitiveMetabolism {
    /// Create a metabolism with default rates.
    #[must_use]
    pub fn default_rates() -> Self {
        Self {
            rates: HashMap::new(),
            global_multiplier: 1.0,
        }
    }

    /// Set a per-operation rate multiplier.
    pub fn set_rate(&mut self, kind: OperationKind, multiplier: f64) {
        self.rates.insert(kind, multiplier);
    }

    /// Compute the cost of an operation of the given kind.
    #[must_use]
    pub fn cost(&self, kind: OperationKind) -> f64 {
        let base = kind.default_cost();
        let rate = self.rates.get(&kind).copied().unwrap_or(1.0);
        base * rate * self.global_multiplier
    }

    /// Set economy mode (halve all costs).
    pub fn set_economy_mode(&mut self) {
        self.global_multiplier = 0.5;
    }

    /// Set performance mode (double all costs, but higher quality).
    pub fn set_performance_mode(&mut self) {
        self.global_multiplier = 2.0;
    }
}

impl Default for CognitiveMetabolism {
    fn default() -> Self {
        Self::default_rates()
    }
}

/// A ledger entry recording an energy transaction.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EnergyTransaction {
    /// When the transaction occurred.
    pub timestamp: DateTime<Utc>,
    /// Kind of transaction.
    pub kind: TransactionKind,
    /// Amount in USD (positive = spend, negative = replenish).
    pub amount: f64,
    /// Running balance after this transaction.
    pub balance_after: f64,
    /// Optional context (task ID, agent ID, etc.).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
}

/// Kind of energy transaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransactionKind {
    /// Energy spent on an operation.
    Spend,
    /// Energy replenished (metabolic recovery).
    Replenish,
    /// Manual budget adjustment.
    Adjustment,
}

/// Available cognitive energy pool with accounting.
///
/// Tracks the current balance, spend history, and replenishment rate.
/// Thread-safe: use behind `Arc<Mutex<EnergyPool>>` for concurrent access.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EnergyPool {
    /// Total budget capacity in USD.
    pub capacity: f64,
    /// Current available balance in USD.
    pub balance: f64,
    /// Per-task cost cap (0.0 = no cap).
    pub per_task_cap: f64,
    /// Metabolism rates.
    pub metabolism: CognitiveMetabolism,
    /// Replenishment rate: fraction of capacity restored per hour.
    pub replenish_rate: f64,
    /// Last time replenishment was applied.
    pub last_replenish: DateTime<Utc>,
    /// Transaction ledger (most recent entries).
    pub ledger: Vec<EnergyTransaction>,
    /// Maximum ledger entries to retain.
    pub max_ledger_entries: usize,
    /// Total spend across all time.
    pub total_spent: f64,
    /// Total replenished across all time.
    pub total_replenished: f64,
}

impl EnergyPool {
    /// Create a new energy pool with the given capacity in USD.
    #[must_use]
    pub fn new(capacity: f64) -> Self {
        Self {
            capacity,
            balance: capacity,
            per_task_cap: 0.0,
            metabolism: CognitiveMetabolism::default(),
            replenish_rate: 0.1,
            last_replenish: Utc::now(),
            ledger: Vec::new(),
            max_ledger_entries: 1000,
            total_spent: 0.0,
            total_replenished: 0.0,
        }
    }

    /// Set the replenishment rate (fraction of capacity per hour).
    pub fn with_replenish_rate(mut self, rate: f64) -> Self {
        self.replenish_rate = rate;
        self
    }

    /// Set the per-task cost cap.
    pub fn with_per_task_cap(mut self, cap: f64) -> Self {
        self.per_task_cap = cap;
        self
    }

    /// Whether there is enough energy for an operation of the given kind.
    #[must_use]
    pub fn can_afford(&self, kind: OperationKind) -> bool {
        let cost = self.metabolism.cost(kind);
        self.balance >= cost
    }

    /// Spend energy on an operation. Returns the cost, or `None` if insufficient balance.
    pub fn spend(&mut self, kind: OperationKind, context: Option<String>) -> Option<f64> {
        let cost = self.metabolism.cost(kind);

        // Check per-task cap
        if self.per_task_cap > 0.0 && cost > self.per_task_cap {
            return None;
        }

        if self.balance < cost {
            return None;
        }

        self.balance -= cost;
        self.total_spent += cost;

        self.record_transaction(TransactionKind::Spend, cost, context);

        Some(cost)
    }

    /// Spend a specific amount of energy. Returns false if insufficient balance.
    pub fn spend_amount(&mut self, amount: f64, context: Option<String>) -> bool {
        if amount < 0.0 || self.balance < amount {
            return false;
        }
        self.balance -= amount;
        self.total_spent += amount;
        self.record_transaction(TransactionKind::Spend, amount, context);
        true
    }

    /// Apply time-based replenishment. Call this periodically (e.g., on heartbeat).
    pub fn replenish(&mut self) {
        let now = Utc::now();
        let elapsed_hours = (now - self.last_replenish).num_seconds() as f64 / 3600.0;
        if elapsed_hours <= 0.0 {
            return;
        }

        let amount =
            (self.capacity * self.replenish_rate * elapsed_hours).min(self.capacity - self.balance);
        if amount > 0.0 {
            self.balance += amount;
            self.total_replenished += amount;
            self.record_transaction(TransactionKind::Replenish, amount, None);
        }
        self.last_replenish = now;
    }

    /// Manual budget adjustment (admin override).
    pub fn adjust(&mut self, amount: f64, context: Option<String>) {
        self.balance = (self.balance + amount).clamp(0.0, self.capacity);
        self.record_transaction(TransactionKind::Adjustment, amount.abs(), context);
    }

    /// Current utilization as a fraction of capacity.
    #[must_use]
    pub fn utilization(&self) -> f64 {
        if self.capacity <= 0.0 {
            return 0.0;
        }
        1.0 - (self.balance / self.capacity)
    }

    /// Whether the pool is critically low (below 10% of capacity).
    #[must_use]
    pub fn is_critical(&self) -> bool {
        self.balance < self.capacity * 0.1
    }

    /// Whether the pool is low (below 25% of capacity).
    #[must_use]
    pub fn is_low(&self) -> bool {
        self.balance < self.capacity * 0.25
    }

    /// Suggested throttle level based on remaining energy.
    /// Returns a multiplier in [0, 1] where 1.0 = no throttle, 0.0 = full stop.
    #[must_use]
    pub fn throttle_level(&self) -> f64 {
        if self.capacity <= 0.0 {
            return 0.0;
        }
        let ratio = self.balance / self.capacity;
        if ratio > 0.5 {
            1.0 // no throttle above 50%
        } else if ratio > 0.1 {
            // Linear ramp from 1.0 at 50% to 0.2 at 10%
            0.2 + (ratio - 0.1) * (0.8 / 0.4)
        } else {
            0.2 // minimum throttle at critical levels
        }
    }

    fn record_transaction(&mut self, kind: TransactionKind, amount: f64, context: Option<String>) {
        let tx = EnergyTransaction {
            timestamp: Utc::now(),
            kind,
            amount,
            balance_after: self.balance,
            context,
        };
        self.ledger.push(tx);

        // Trim old entries.
        if self.ledger.len() > self.max_ledger_entries {
            let excess = self.ledger.len() - self.max_ledger_entries;
            self.ledger.drain(..excess);
        }
    }
}

impl Default for EnergyPool {
    fn default() -> Self {
        Self::new(50.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn energy_pool_creation() {
        let pool = EnergyPool::new(100.0);
        assert_eq!(pool.capacity, 100.0);
        assert_eq!(pool.balance, 100.0);
        assert_eq!(pool.utilization(), 0.0);
        assert!(!pool.is_low());
        assert!(!pool.is_critical());
    }

    #[test]
    fn spend_reduces_balance() {
        let mut pool = EnergyPool::new(10.0);
        let cost = pool.spend(OperationKind::LlmCall, None).unwrap();
        assert!(cost > 0.0);
        assert!(pool.balance < 10.0);
        assert!(pool.total_spent > 0.0);
        assert_eq!(pool.ledger.len(), 1);
    }

    #[test]
    fn spend_fails_when_insufficient() {
        let mut pool = EnergyPool::new(0.001);
        pool.balance = 0.0;
        assert!(pool.spend(OperationKind::LlmCall, None).is_none());
    }

    #[test]
    fn can_afford_checks() {
        let pool = EnergyPool::new(100.0);
        assert!(pool.can_afford(OperationKind::LlmCall));

        let mut low = EnergyPool::new(0.001);
        low.balance = 0.0;
        assert!(!low.can_afford(OperationKind::LlmCall));
    }

    #[test]
    fn spend_amount_specific() {
        let mut pool = EnergyPool::new(10.0);
        assert!(pool.spend_amount(5.0, Some("task-1".into())));
        assert_eq!(pool.balance, 5.0);
        assert!(!pool.spend_amount(6.0, None));
    }

    #[test]
    fn per_task_cap_enforced() {
        let mut pool = EnergyPool::new(100.0).with_per_task_cap(0.005);
        // LlmCall costs 0.01 which exceeds 0.005 cap
        assert!(pool.spend(OperationKind::LlmCall, None).is_none());
        // ToolCall costs 0.001 which is within cap
        assert!(pool.spend(OperationKind::ToolCall, None).is_some());
    }

    #[test]
    fn metabolism_cost_multipliers() {
        let mut metabolism = CognitiveMetabolism::default();
        let base = OperationKind::LlmCall.default_cost();

        // Default: 1.0x
        assert_eq!(metabolism.cost(OperationKind::LlmCall), base);

        // Set 2x rate for LLM calls
        metabolism.set_rate(OperationKind::LlmCall, 2.0);
        assert_eq!(metabolism.cost(OperationKind::LlmCall), base * 2.0);

        // Global multiplier stacks
        metabolism.global_multiplier = 0.5;
        assert_eq!(metabolism.cost(OperationKind::LlmCall), base * 2.0 * 0.5);
    }

    #[test]
    fn economy_and_performance_modes() {
        let mut metabolism = CognitiveMetabolism::default();
        let base = OperationKind::Research.default_cost();

        metabolism.set_economy_mode();
        assert_eq!(metabolism.cost(OperationKind::Research), base * 0.5);

        metabolism.set_performance_mode();
        assert_eq!(metabolism.cost(OperationKind::Research), base * 2.0);
    }

    #[test]
    fn utilization_and_levels() {
        let mut pool = EnergyPool::new(100.0);
        assert_eq!(pool.utilization(), 0.0);
        assert!(!pool.is_low());
        assert!(!pool.is_critical());

        pool.balance = 20.0;
        assert!((pool.utilization() - 0.8).abs() < 0.001);
        assert!(pool.is_low());
        assert!(!pool.is_critical());

        pool.balance = 5.0;
        assert!(pool.is_critical());
        assert!(pool.is_low());
    }

    #[test]
    fn throttle_level_gradual() {
        let mut pool = EnergyPool::new(100.0);

        // Full balance: no throttle
        assert_eq!(pool.throttle_level(), 1.0);

        // 30% balance: moderate throttle
        pool.balance = 30.0;
        let throttle = pool.throttle_level();
        assert!(throttle > 0.2 && throttle < 1.0);

        // 5% balance: minimum throttle
        pool.balance = 5.0;
        assert_eq!(pool.throttle_level(), 0.2);

        // 0 capacity: full stop
        let empty = EnergyPool::new(0.0);
        assert_eq!(empty.throttle_level(), 0.0);
    }

    #[test]
    fn adjust_clamps_to_capacity() {
        let mut pool = EnergyPool::new(100.0);
        pool.adjust(50.0, None);
        assert_eq!(pool.balance, 100.0); // already at capacity

        pool.balance = 50.0;
        pool.adjust(-60.0, None);
        // balance + (-60.0) = -10.0 clamped to 0.0
        assert_eq!(pool.balance, 0.0);
    }

    #[test]
    fn ledger_trimming() {
        let mut pool = EnergyPool::new(1_000_000.0);
        pool.max_ledger_entries = 5;

        for i in 0..10 {
            pool.spend_amount(0.001, Some(format!("tx-{}", i)));
        }

        assert!(pool.ledger.len() <= 5);
    }

    #[test]
    fn operation_kind_labels() {
        assert_eq!(OperationKind::LlmCall.label(), "llm_call");
        assert_eq!(OperationKind::GateRun.label(), "gate_run");
        assert!(OperationKind::LlmCall.default_cost() > 0.0);
    }

    #[test]
    fn serde_roundtrip_pool() {
        let mut pool = EnergyPool::new(50.0);
        pool.spend(OperationKind::ToolCall, Some("test".into()));
        let json = serde_json::to_string(&pool).unwrap();
        let back: EnergyPool = serde_json::from_str(&json).unwrap();
        assert_eq!(back.capacity, pool.capacity);
        assert_eq!(back.balance, pool.balance);
        assert_eq!(back.ledger.len(), pool.ledger.len());
    }
}
