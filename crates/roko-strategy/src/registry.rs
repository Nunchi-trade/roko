//! Strategy registry + loader.
//!
//! Ports `sdk/strategy_sdk/registry.py` + `loader.py`. Strategies register an
//! id + factory; `load(id, params)` constructs a boxed `Strategy` trait object.

use std::collections::HashMap;

use serde_json::Value as JsonValue;

use crate::strategies::*;
use crate::strategy::{Strategy, StrategyError};

/// Factory that constructs a strategy from a parameter map. Each registered
/// strategy owns its own parameter parsing to keep the registry generic.
pub type StrategyFactory = fn(&HashMap<String, JsonValue>) -> Result<Box<dyn Strategy>, StrategyError>;

/// In-process strategy registry. Use [`StrategyRegistry::default`] to get the
/// full built-in set; extend with [`StrategyRegistry::register`] at startup.
pub struct StrategyRegistry {
    factories: HashMap<String, StrategyFactory>,
}

impl Default for StrategyRegistry {
    fn default() -> Self {
        let mut r = Self {
            factories: HashMap::new(),
        };
        r.register("simple_mm", factory_simple_mm);
        r.register("grid_mm", factory_grid_mm);
        r.register("avellaneda_mm", factory_avellaneda_mm);
        r.register("mean_reversion", factory_mean_reversion);
        r.register("momentum_breakout", factory_momentum_breakout);
        r.register("trend_follower", factory_trend_follower);
        r.register("funding_momentum", factory_funding_momentum);
        r.register("basis_arb", factory_basis_arb);
        r.register("aggressive_taker", factory_aggressive_taker);
        r.register("hedge_agent", factory_hedge_agent);
        r.register("oi_divergence", factory_oi_divergence);
        r
    }
}

impl StrategyRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, id: &str, factory: StrategyFactory) {
        self.factories.insert(id.into(), factory);
    }

    pub fn load(&self, id: &str, params: &HashMap<String, JsonValue>) -> Result<Box<dyn Strategy>, StrategyError> {
        self.factories
            .get(id)
            .ok_or_else(|| StrategyError::NotFound(id.into()))
            .and_then(|f| f(params))
    }

    pub fn ids(&self) -> Vec<&str> {
        let mut v: Vec<&str> = self.factories.keys().map(String::as_str).collect();
        v.sort_unstable();
        v
    }
}

// ---------------------------------------------------------------- helpers

fn f(params: &HashMap<String, JsonValue>, key: &str, default: f64) -> f64 {
    params.get(key).and_then(JsonValue::as_f64).unwrap_or(default)
}

fn u(params: &HashMap<String, JsonValue>, key: &str, default: u64) -> u64 {
    params.get(key).and_then(JsonValue::as_u64).unwrap_or(default)
}

// ---------------------------------------------------------------- factories

fn factory_simple_mm(p: &HashMap<String, JsonValue>) -> Result<Box<dyn Strategy>, StrategyError> {
    Ok(Box::new(SimpleMMStrategy::new(f(p, "spread_bps", 10.0), f(p, "size", 1.0))))
}

fn factory_grid_mm(p: &HashMap<String, JsonValue>) -> Result<Box<dyn Strategy>, StrategyError> {
    Ok(Box::new(GridMMStrategy::new(
        f(p, "grid_spacing_bps", 10.0),
        u(p, "num_levels", 5) as u32,
        f(p, "size_per_level", 0.5),
        f(p, "max_position", 5.0),
    )))
}

fn factory_avellaneda_mm(p: &HashMap<String, JsonValue>) -> Result<Box<dyn Strategy>, StrategyError> {
    Ok(Box::new(AvellanedaStoikovStrategy::new(
        f(p, "gamma", 0.1),
        f(p, "k", 1.5),
        f(p, "base_size", 1.0),
        f(p, "max_inventory", 10.0),
        f(p, "min_spread_bps", 5.0),
        f(p, "max_spread_bps", 200.0),
        u(p, "vol_window", 30) as usize,
    )))
}

fn factory_mean_reversion(p: &HashMap<String, JsonValue>) -> Result<Box<dyn Strategy>, StrategyError> {
    Ok(Box::new(MeanReversionStrategy::new(
        u(p, "window", 20) as usize,
        f(p, "threshold_bps", 30.0),
        f(p, "size", 1.0),
    )))
}

fn factory_momentum_breakout(p: &HashMap<String, JsonValue>) -> Result<Box<dyn Strategy>, StrategyError> {
    Ok(Box::new(MomentumBreakoutStrategy::new(
        u(p, "lookback", 20) as usize,
        f(p, "breakout_threshold_bps", 50.0),
        f(p, "volume_surge_mult", 2.0),
        f(p, "trailing_stop_bps", 30.0),
        f(p, "size", 1.0),
    )))
}

fn factory_trend_follower(p: &HashMap<String, JsonValue>) -> Result<Box<dyn Strategy>, StrategyError> {
    Ok(Box::new(TrendFollowerStrategy::new(f(p, "size", 1.0))))
}

fn factory_funding_momentum(p: &HashMap<String, JsonValue>) -> Result<Box<dyn Strategy>, StrategyError> {
    Ok(Box::new(FundingMomentumStrategy::new(f(p, "size", 1.0))))
}

fn factory_basis_arb(p: &HashMap<String, JsonValue>) -> Result<Box<dyn Strategy>, StrategyError> {
    Ok(Box::new(BasisArbStrategy::new(
        f(p, "basis_threshold_bps", 5.0),
        f(p, "size", 1.0),
        u(p, "funding_window", 10) as usize,
    )))
}

fn factory_aggressive_taker(p: &HashMap<String, JsonValue>) -> Result<Box<dyn Strategy>, StrategyError> {
    Ok(Box::new(AggressiveTakerStrategy::new(
        f(p, "size", 2.0),
        u(p, "skip_ticks", 0) as u32,
        f(p, "bias_amplitude", 0.35),
        u(p, "bias_period", 4) as u32,
    )))
}

fn factory_hedge_agent(p: &HashMap<String, JsonValue>) -> Result<Box<dyn Strategy>, StrategyError> {
    Ok(Box::new(HedgeAgent::new(
        f(p, "inventory_threshold", 3.0),
        f(p, "urgency_factor", 0.5),
        f(p, "max_hedge_size", 5.0),
        f(p, "slippage_bps", 10.0),
    )))
}

fn factory_oi_divergence(p: &HashMap<String, JsonValue>) -> Result<Box<dyn Strategy>, StrategyError> {
    Ok(Box::new(OIDivergenceStrategy::new(f(p, "size", 1.0))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_has_11_builtins() {
        let r = StrategyRegistry::default();
        assert_eq!(r.ids().len(), 11);
    }

    #[test]
    fn load_simple_mm_from_defaults() {
        let r = StrategyRegistry::default();
        let s = r.load("simple_mm", &HashMap::new()).unwrap();
        assert_eq!(s.strategy_id(), "simple_mm");
    }

    #[test]
    fn unknown_id_errors() {
        let r = StrategyRegistry::default();
        assert!(r.load("does_not_exist", &HashMap::new()).is_err());
    }
}
