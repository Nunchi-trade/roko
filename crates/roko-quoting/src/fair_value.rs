//! Composite fair-value calculator. Ports `quoting_engine/fair_value.py`.

use crate::config::FairValueWeights;

#[derive(Debug, Clone, Copy)]
pub struct FairValueCalculator {
    weights: FairValueWeights,
}

impl FairValueCalculator {
    #[must_use]
    pub const fn new(weights: FairValueWeights) -> Self {
        Self { weights }
    }

    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn compute(
        &self,
        oracle_price: f64,
        bid: f64,
        ask: f64,
        mut external_ref: f64,
        inventory_term: f64,
        microprice_override: Option<f64>,
        oracle_weight_override: Option<f64>,
    ) -> f64 {
        if oracle_price <= 0.0 {
            return 0.0;
        }
        if external_ref <= 0.0 {
            external_ref = oracle_price;
        }
        let microprice = match microprice_override {
            Some(m) if m > 0.0 => m,
            _ => {
                let spread = ask - bid;
                if spread > 0.0 && bid > 0.0 && ask > 0.0 {
                    let bid_weight = ((ask - oracle_price) / spread).clamp(0.0, 1.0);
                    let ask_weight = ((oracle_price - bid) / spread).clamp(0.0, 1.0);
                    bid * ask_weight + ask * bid_weight
                } else {
                    oracle_price
                }
            }
        };

        if let Some(w_oracle) = oracle_weight_override {
            let remaining = 1.0 - w_oracle;
            let other_total = self.weights.w_external + self.weights.w_microprice + self.weights.w_inventory;
            let (w_ext, w_micro, w_inv) = if other_total > 0.0 {
                let scale = remaining / other_total;
                (
                    self.weights.w_external * scale,
                    self.weights.w_microprice * scale,
                    self.weights.w_inventory * scale,
                )
            } else {
                (0.0, 0.0, 0.0)
            };
            w_oracle * oracle_price + w_ext * external_ref + w_micro * microprice + w_inv * inventory_term
        } else {
            self.weights.w_oracle * oracle_price
                + self.weights.w_external * external_ref
                + self.weights.w_microprice * microprice
                + self.weights.w_inventory * inventory_term
        }
    }
}
