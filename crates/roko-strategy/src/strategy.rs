//! Core [`Strategy`] trait + shared context / decision types.
//!
//! Ports `sdk/strategy_sdk/base.py` and `common.models.StrategyDecision` from
//! `Nunchi-trade/offchainservices-agent`.

use roko_venue::{MarketSnapshot, OrderSide};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

/// Order-type discriminator emitted by strategies. Matches the Python
/// `StrategyDecision.order_type` string values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum StrategyOrderType {
    Gtc,
    Ioc,
    Alo,
}

impl Default for StrategyOrderType {
    fn default() -> Self {
        Self::Gtc
    }
}

impl StrategyOrderType {
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Gtc => "Gtc",
            Self::Ioc => "Ioc",
            Self::Alo => "Alo",
        }
    }
}

impl From<StrategyOrderType> for roko_venue::TimeInForce {
    fn from(v: StrategyOrderType) -> Self {
        match v {
            StrategyOrderType::Gtc => Self::Gtc,
            StrategyOrderType::Ioc => Self::Ioc,
            StrategyOrderType::Alo => Self::Alo,
        }
    }
}

/// Rich context passed to strategies each tick. Matches
/// `sdk.strategy_sdk.base.StrategyContext`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyContext {
    pub snapshot: MarketSnapshot,
    pub position_qty: f64,
    pub position_notional: f64,
    pub unrealized_pnl: f64,
    pub realized_pnl: f64,
    pub reduce_only: bool,
    pub safe_mode: bool,
    pub round_number: u64,
    pub daily_drawdown_pct: f64,
    pub meta: HashMap<String, serde_json::Value>,
}

impl Default for StrategyContext {
    fn default() -> Self {
        Self {
            snapshot: MarketSnapshot::default(),
            position_qty: 0.0,
            position_notional: 0.0,
            unrealized_pnl: 0.0,
            realized_pnl: 0.0,
            reduce_only: false,
            safe_mode: false,
            round_number: 0,
            daily_drawdown_pct: 0.0,
            meta: HashMap::new(),
        }
    }
}

/// A single order instruction emitted by a strategy. Mirrors
/// `common.models.StrategyDecision`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyDecision {
    pub action: String,
    pub instrument: String,
    pub side: OrderSide,
    pub size: f64,
    pub limit_price: f64,
    pub order_type: StrategyOrderType,
    pub meta: HashMap<String, serde_json::Value>,
}

impl StrategyDecision {
    /// Convenient constructor for the common "place limit order" case.
    #[must_use]
    pub fn place(
        instrument: impl Into<String>,
        side: OrderSide,
        size: f64,
        limit_price: f64,
    ) -> Self {
        Self {
            action: "place_order".into(),
            instrument: instrument.into(),
            side,
            size,
            limit_price,
            order_type: StrategyOrderType::Gtc,
            meta: HashMap::new(),
        }
    }

    #[must_use]
    pub fn with_order_type(mut self, ot: StrategyOrderType) -> Self {
        self.order_type = ot;
        self
    }

    #[must_use]
    pub fn with_meta(mut self, key: impl Into<String>, value: impl Into<serde_json::Value>) -> Self {
        self.meta.insert(key.into(), value.into());
        self
    }
}

#[derive(Debug, Error)]
pub enum StrategyError {
    #[error("invalid strategy parameter: {0}")]
    InvalidParam(String),
    #[error("strategy {0} not found in registry")]
    NotFound(String),
    #[error("other: {0}")]
    Other(String),
}

/// A trading strategy. One implementation per distinct algorithm.
///
/// Synchronous because strategies are expected to be fast + side-effect-free —
/// all I/O happens in the venue / engine / event-bus layer.
pub trait Strategy: Send + Sync {
    /// Stable identifier (e.g. `"simple_mm"`). Used by the registry and for
    /// telemetry.
    fn strategy_id(&self) -> &str;

    /// Called each tick with current market data + context. Returns zero-or-
    /// more [`StrategyDecision`]s describing orders to submit this round.
    fn on_tick(&mut self, snapshot: &MarketSnapshot, context: &StrategyContext) -> Vec<StrategyDecision>;
}
