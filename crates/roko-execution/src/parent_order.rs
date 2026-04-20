//! Parent-order model for multi-tick execution. Ports `execution/parent_order.py`.

use roko_venue::OrderSide;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExecutionAlgo {
    Twap,
    Immediate,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParentOrder {
    pub order_id: String,
    pub instrument: String,
    pub side: OrderSide,
    pub target_qty: f64,
    pub algo: ExecutionAlgo,
    pub duration_ticks: u32,
    pub urgency: f64,
    pub filled_qty: f64,
    pub status: String,
    pub ticks_elapsed: u32,
    pub created_at_ms: i64,
}

impl ParentOrder {
    #[must_use]
    pub fn new(instrument: impl Into<String>, side: OrderSide, target_qty: f64, duration_ticks: u32) -> Self {
        Self {
            order_id: Uuid::new_v4().to_string(),
            instrument: instrument.into(),
            side,
            target_qty,
            algo: ExecutionAlgo::Twap,
            duration_ticks,
            urgency: 0.7,
            filled_qty: 0.0,
            status: "active".into(),
            ticks_elapsed: 0,
            created_at_ms: 0,
        }
    }

    #[must_use]
    pub fn remaining_qty(&self) -> f64 {
        (self.target_qty - self.filled_qty).max(0.0)
    }

    #[must_use]
    pub fn progress(&self) -> f64 {
        if self.target_qty <= 0.0 {
            0.0
        } else {
            self.filled_qty / self.target_qty
        }
    }

    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.status != "active"
    }

    pub fn record_fill(&mut self, qty: f64, _price: f64, _timestamp_ms: i64) {
        self.filled_qty += qty;
        if self.remaining_qty() <= 0.0 {
            self.status = "complete".into();
        }
    }
}
