//! TWAP slicing. Ports `execution/twap.py`.

use std::collections::HashMap;

use rand::{Rng, SeedableRng};
use rand::rngs::SmallRng;
use roko_venue::{MarketSnapshot, OrderSide};
use serde::{Deserialize, Serialize};

use crate::parent_order::ParentOrder;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChildSlice {
    pub parent_order_id: String,
    pub instrument: String,
    pub side: OrderSide,
    pub size: f64,
    pub price: f64,
}

pub struct TwapExecutor {
    active: HashMap<String, ParentOrder>,
    rng: SmallRng,
}

impl TwapExecutor {
    #[must_use]
    pub fn new() -> Self {
        Self {
            active: HashMap::new(),
            rng: SmallRng::from_entropy(),
        }
    }

    #[must_use]
    pub fn with_seed(seed: u64) -> Self {
        Self {
            active: HashMap::new(),
            rng: SmallRng::seed_from_u64(seed),
        }
    }

    pub fn submit(&mut self, order: ParentOrder) {
        self.active.insert(order.order_id.clone(), order);
    }

    pub fn on_tick(&mut self, snap: &MarketSnapshot) -> Vec<ChildSlice> {
        let mut slices = Vec::new();
        let mut completed = Vec::new();
        let keys: Vec<String> = self.active.keys().cloned().collect();
        for k in keys {
            let order = self.active.get_mut(&k).expect("key exists");
            if order.is_complete() {
                completed.push(k.clone());
                continue;
            }
            order.ticks_elapsed += 1;
            if let Some(slice) = compute_slice(order, snap, &mut self.rng) {
                slices.push(slice);
            }
        }
        for k in completed {
            self.active.remove(&k);
        }
        slices
    }

    pub fn record_fill(&mut self, order_id: &str, qty: f64, price: f64, ts: i64) {
        if let Some(order) = self.active.get_mut(order_id) {
            order.record_fill(qty, price, ts);
        }
    }

    #[must_use]
    pub fn active_count(&self) -> usize {
        self.active.len()
    }
}

impl Default for TwapExecutor {
    fn default() -> Self {
        Self::new()
    }
}

fn compute_slice(order: &mut ParentOrder, snap: &MarketSnapshot, rng: &mut SmallRng) -> Option<ChildSlice> {
    let remaining_ticks = (order.duration_ticks.saturating_sub(order.ticks_elapsed)).max(1) as f64;
    let base_slice = order.remaining_qty() / remaining_ticks;
    let urgency_factor = 1.0 + order.urgency * 0.5;
    let mut slice_qty = (base_slice * urgency_factor).min(order.remaining_qty());

    let skip_prob = (0.2 * (1.0 - order.urgency)).max(0.0);
    if rng.r#gen::<f64>() < skip_prob {
        return None;
    }

    let jitter = 1.0 + rng.gen_range(-0.15..0.15);
    slice_qty = (slice_qty * jitter).min(order.remaining_qty());
    if slice_qty <= 0.0 {
        return None;
    }

    let price = match order.side {
        OrderSide::Buy => if snap.ask > 0.0 { snap.ask } else { snap.mid_price },
        OrderSide::Sell => if snap.bid > 0.0 { snap.bid } else { snap.mid_price },
    };

    Some(ChildSlice {
        parent_order_id: order.order_id.clone(),
        instrument: order.instrument.clone(),
        side: order.side,
        size: (slice_qty * 1_000_000.0).round() / 1_000_000.0,
        price,
    })
}
