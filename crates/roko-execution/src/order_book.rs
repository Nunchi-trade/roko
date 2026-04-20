//! Managed order book — holds + ticks bracket/conditional/pegged orders.
//! Ports `execution/order_book.py`.

use std::collections::HashMap;

use roko_strategy::StrategyDecision;
use roko_venue::MarketSnapshot;

use crate::order_types::ManagedOrder;

#[derive(Debug, Default)]
pub struct ManagedOrderBook {
    orders: HashMap<String, ManagedOrder>,
}

impl ManagedOrderBook {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, order: ManagedOrder) {
        self.orders.insert(order.id().to_string(), order);
    }

    pub fn remove(&mut self, order_id: &str) {
        self.orders.remove(order_id);
    }

    pub fn on_tick(&mut self, snap: &MarketSnapshot) -> Vec<StrategyDecision> {
        let mut decisions = Vec::new();
        let mut to_remove = Vec::new();
        for (oid, order) in &mut self.orders {
            if let Some(decision) = order.on_tick(snap) {
                decisions.push(decision);
            }
            if !matches!(order.status(), "active" | "pending") {
                to_remove.push(oid.clone());
            }
        }
        for oid in to_remove {
            self.orders.remove(&oid);
        }
        decisions
    }

    #[must_use]
    pub fn count(&self) -> usize {
        self.orders.len()
    }

    #[must_use]
    pub fn get(&self, order_id: &str) -> Option<&ManagedOrder> {
        self.orders.get(order_id)
    }
}
