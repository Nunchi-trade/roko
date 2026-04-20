//! Managed order types — bracket, conditional, pegged. Ports `execution/order_types.py`.

use roko_strategy::{StrategyDecision, StrategyOrderType};
use roko_venue::{MarketSnapshot, OrderSide};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Long,
    Short,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BracketOrder {
    pub order_id: String,
    pub instrument: String,
    pub direction: Direction,
    pub entry_price: f64,
    pub entry_size: f64,
    pub take_profit_price: f64,
    pub stop_loss_price: f64,
    pub status: String,
}

impl BracketOrder {
    pub fn on_tick(&mut self, snap: &MarketSnapshot) -> Option<StrategyDecision> {
        if self.status != "active" {
            return None;
        }
        let mid = snap.mid_price;
        if mid <= 0.0 {
            return None;
        }
        let close_side = match self.direction {
            Direction::Long => OrderSide::Sell,
            Direction::Short => OrderSide::Buy,
        };
        let tp_hit = matches!(self.direction, Direction::Long) && mid >= self.take_profit_price
            || matches!(self.direction, Direction::Short) && mid <= self.take_profit_price;
        let sl_hit = matches!(self.direction, Direction::Long) && mid <= self.stop_loss_price
            || matches!(self.direction, Direction::Short) && mid >= self.stop_loss_price;

        if tp_hit {
            self.status = "tp_triggered".into();
            return Some(exit_decision(&self.instrument, close_side, self.entry_size, mid, "take_profit", &self.order_id));
        }
        if sl_hit {
            self.status = "sl_triggered".into();
            return Some(exit_decision(&self.instrument, close_side, self.entry_size, mid, "stop_loss", &self.order_id));
        }
        None
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConditionalOrder {
    pub order_id: String,
    pub instrument: String,
    pub trigger_price: f64,
    pub trigger_above: bool,
    pub child_side: OrderSide,
    pub child_size: f64,
    pub status: String,
    pub expiry_ms: i64,
}

impl ConditionalOrder {
    pub fn on_tick(&mut self, snap: &MarketSnapshot) -> Option<StrategyDecision> {
        if self.status != "pending" {
            return None;
        }
        let mid = snap.mid_price;
        if mid <= 0.0 {
            return None;
        }
        if self.expiry_ms > 0 && snap.timestamp_ms > self.expiry_ms {
            self.status = "expired".into();
            return None;
        }
        let triggered = if self.trigger_above {
            mid >= self.trigger_price
        } else {
            mid <= self.trigger_price
        };
        if !triggered {
            return None;
        }
        self.status = "triggered".into();
        Some(
            StrategyDecision::place(self.instrument.clone(), self.child_side, self.child_size, mid)
                .with_order_type(StrategyOrderType::Ioc)
                .with_meta("trigger", "conditional")
                .with_meta("conditional_id", self.order_id.clone()),
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeggedOrder {
    pub order_id: String,
    pub instrument: String,
    pub side: OrderSide,
    pub size: f64,
    pub offset_bps: f64,
    pub status: String,
    pub max_ticks: u32,
    pub ticks_elapsed: u32,
}

impl PeggedOrder {
    pub fn on_tick(&mut self, snap: &MarketSnapshot) -> Option<StrategyDecision> {
        if self.status != "active" {
            return None;
        }
        self.ticks_elapsed += 1;
        if self.max_ticks > 0 && self.ticks_elapsed > self.max_ticks {
            self.status = "expired".into();
            return None;
        }
        let mid = snap.mid_price;
        if mid <= 0.0 {
            return None;
        }
        let offset = mid * self.offset_bps / 10_000.0;
        let price = match self.side {
            OrderSide::Buy => mid - offset,
            OrderSide::Sell => mid + offset,
        };
        Some(
            StrategyDecision::place(self.instrument.clone(), self.side, self.size, round6(price))
                .with_meta("trigger", "pegged")
                .with_meta("pegged_id", self.order_id.clone()),
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ManagedOrder {
    Bracket(BracketOrder),
    Conditional(ConditionalOrder),
    Pegged(PeggedOrder),
}

impl ManagedOrder {
    #[must_use]
    pub fn id(&self) -> &str {
        match self {
            Self::Bracket(o) => &o.order_id,
            Self::Conditional(o) => &o.order_id,
            Self::Pegged(o) => &o.order_id,
        }
    }

    #[must_use]
    pub fn status(&self) -> &str {
        match self {
            Self::Bracket(o) => &o.status,
            Self::Conditional(o) => &o.status,
            Self::Pegged(o) => &o.status,
        }
    }

    pub fn on_tick(&mut self, snap: &MarketSnapshot) -> Option<StrategyDecision> {
        match self {
            Self::Bracket(o) => o.on_tick(snap),
            Self::Conditional(o) => o.on_tick(snap),
            Self::Pegged(o) => o.on_tick(snap),
        }
    }
}

fn exit_decision(instrument: &str, side: OrderSide, size: f64, mid: f64, trigger: &str, bracket_id: &str) -> StrategyDecision {
    StrategyDecision::place(instrument.to_string(), side, size, mid)
        .with_order_type(StrategyOrderType::Ioc)
        .with_meta("trigger", trigger.to_string())
        .with_meta("bracket_id", bracket_id.to_string())
}

fn round6(x: f64) -> f64 {
    (x * 1_000_000.0).round() / 1_000_000.0
}
