//! Smart order routing + ALO stats. Ports `execution/routing.py`.

use roko_strategy::{StrategyDecision, StrategyOrderType};
use roko_venue::{MarketSnapshot, VenueCapabilities};
use serde::{Deserialize, Serialize};

/// Tracks ALO routing metrics for REFLECT integration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AloStats {
    pub alo_attempts: u64,
    pub alo_successes: u64,
    pub alo_fallbacks: u64,
    pub gtc_orders: u64,
    pub ioc_orders: u64,
    pub estimated_maker_rebate_usd: f64,
}

impl AloStats {
    pub fn record_alo_attempt(&mut self, success: bool, size_usd: f64, rebate_bps: f64) {
        self.alo_attempts += 1;
        if success {
            self.alo_successes += 1;
            self.estimated_maker_rebate_usd += size_usd * rebate_bps / 10_000.0;
        } else {
            self.alo_fallbacks += 1;
        }
    }

    pub fn record_order(&mut self, tif: StrategyOrderType) {
        match tif {
            StrategyOrderType::Gtc => self.gtc_orders += 1,
            StrategyOrderType::Ioc => self.ioc_orders += 1,
            StrategyOrderType::Alo => {}
        }
    }

    #[must_use]
    pub fn alo_success_rate(&self) -> f64 {
        if self.alo_attempts == 0 {
            0.0
        } else {
            self.alo_successes as f64 / self.alo_attempts as f64 * 100.0
        }
    }
}

/// Smart TIF selection — ports `OrderRouter.route`.
pub struct OrderRouter {
    caps: VenueCapabilities,
    stats: AloStats,
}

impl OrderRouter {
    #[must_use]
    pub const fn new(caps: VenueCapabilities) -> Self {
        Self { caps, stats: AloStats {
            alo_attempts: 0,
            alo_successes: 0,
            alo_fallbacks: 0,
            gtc_orders: 0,
            ioc_orders: 0,
            estimated_maker_rebate_usd: 0.0,
        }}
    }

    #[must_use]
    pub fn route(&self, decision: &StrategyDecision, snapshot: &MarketSnapshot, urgency: f64) -> StrategyOrderType {
        if !self.caps.supports_alo {
            return if matches!(decision.order_type, StrategyOrderType::Alo) {
                StrategyOrderType::Gtc
            } else {
                decision.order_type
            };
        }
        if urgency >= 0.8 {
            return StrategyOrderType::Ioc;
        }
        if snapshot.spread_bps > 5.0 && urgency < 0.5 {
            return StrategyOrderType::Alo;
        }
        if snapshot.spread_bps < 2.0 {
            return StrategyOrderType::Gtc;
        }
        decision.order_type
    }

    pub fn stats_mut(&mut self) -> &mut AloStats {
        &mut self.stats
    }

    #[must_use]
    pub const fn stats(&self) -> &AloStats {
        &self.stats
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_venue::OrderSide;

    fn snap_spread(bps: f64) -> MarketSnapshot {
        MarketSnapshot {
            instrument: "ETH-PERP".into(),
            mid_price: 2500.0,
            bid: 2499.5,
            ask: 2500.5,
            spread_bps: bps,
            ..Default::default()
        }
    }

    #[test]
    fn wide_spread_low_urgency_alo() {
        let r = OrderRouter::new(VenueCapabilities {
            supports_alo: true,
            ..Default::default()
        });
        let d = StrategyDecision::place("ETH-PERP", OrderSide::Buy, 0.1, 2500.0);
        let tif = r.route(&d, &snap_spread(10.0), 0.2);
        assert!(matches!(tif, StrategyOrderType::Alo));
    }

    #[test]
    fn high_urgency_ioc() {
        let r = OrderRouter::new(VenueCapabilities {
            supports_alo: true,
            ..Default::default()
        });
        let d = StrategyDecision::place("ETH-PERP", OrderSide::Sell, 0.1, 2500.0);
        let tif = r.route(&d, &snap_spread(10.0), 0.9);
        assert!(matches!(tif, StrategyOrderType::Ioc));
    }

    #[test]
    fn no_alo_support_coerces_alo_to_gtc() {
        let r = OrderRouter::new(VenueCapabilities::default());
        let d = StrategyDecision::place("ETH-PERP", OrderSide::Buy, 0.1, 2500.0)
            .with_order_type(StrategyOrderType::Alo);
        assert!(matches!(r.route(&d, &snap_spread(3.0), 0.5), StrategyOrderType::Gtc));
    }
}
