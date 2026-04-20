//! Inventory-aware skew + soft/hard cap state. Ports
//! `quoting_engine/inventory.py`.

use crate::config::{SkewMode, SkewParams};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InventoryState {
    Normal,
    SoftBreach,
    HardBreach,
}

#[derive(Debug, Clone, Copy)]
pub struct MicroClipOrder {
    pub side: &'static str,
    pub size: f64,
}

#[derive(Debug, Clone, Copy)]
pub struct InventorySkewer {
    p: SkewParams,
}

impl InventorySkewer {
    #[must_use]
    pub const fn new(p: SkewParams) -> Self {
        Self { p }
    }

    #[must_use]
    pub fn price_skew(&self, fv: f64, inventory: f64, sigma_price: f64) -> f64 {
        if matches!(self.p.mode, SkewMode::Size) {
            return fv;
        }
        if self.p.inv_limit <= 0.0 {
            return fv;
        }
        let skew = -self.p.k_inv * (inventory / self.p.inv_limit) * sigma_price;
        fv + skew
    }

    #[must_use]
    pub fn size_skew(&self, base_bid: f64, base_ask: f64, inventory: f64) -> (f64, f64) {
        if matches!(self.p.mode, SkewMode::Price) {
            return (base_bid, base_ask);
        }
        if self.p.inv_limit <= 0.0 {
            return (base_bid, base_ask);
        }
        let util = (inventory / self.p.inv_limit).clamp(-1.0, 1.0);
        let factor = self.p.size_skew_factor;
        let bid_mult = (1.0 - factor * util).max(0.0);
        let ask_mult = (1.0 + factor * util).max(0.0);
        (round6(base_bid * bid_mult), round6(base_ask * ask_mult))
    }

    #[must_use]
    pub fn effective_limit(&self) -> f64 {
        if self.p.hard_cap > 0.0 {
            self.p.hard_cap
        } else {
            self.p.inv_limit
        }
    }

    #[must_use]
    pub fn inventory_state(&self, inventory: f64) -> InventoryState {
        let abs = inventory.abs();
        let hard = if self.p.hard_cap > 0.0 { self.p.hard_cap } else { self.p.inv_limit };
        let soft = if self.p.soft_cap > 0.0 { self.p.soft_cap } else { hard };
        if abs >= hard {
            InventoryState::HardBreach
        } else if abs >= soft {
            InventoryState::SoftBreach
        } else {
            InventoryState::Normal
        }
    }

    #[must_use]
    pub fn micro_clip_order(&self, inventory: f64, tick_count: u64) -> Option<MicroClipOrder> {
        if self.p.micro_clip_size <= 0.0 {
            return None;
        }
        if self.inventory_state(inventory) != InventoryState::SoftBreach {
            return None;
        }
        if self.p.micro_clip_interval == 0 || tick_count % u64::from(self.p.micro_clip_interval) != 0 {
            return None;
        }
        let side = if inventory > 0.0 { "sell" } else { "buy" };
        let size = self.p.micro_clip_size.min(inventory.abs());
        Some(MicroClipOrder { side, size })
    }
}

fn round6(x: f64) -> f64 {
    (x * 1_000_000.0).round() / 1_000_000.0
}
