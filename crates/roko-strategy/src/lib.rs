//! Strategy trait + 17 strategy implementations ported from
//! `Nunchi-trade/offchainservices-agent`'s `strategies/` directory.
//!
//! Strategies depend only on the [`Strategy`] trait + [`StrategyContext`] +
//! [`StrategyDecision`]; no venue-specific types leak through. Use with
//! `roko-venue::VenueAdapter` via `roko-trading-agent::TradingAgent`.
//!
//! See `plans/P08-trading-surface.md` T3 for porting notes.

#![forbid(unsafe_code)]
#![allow(missing_docs)]

mod risk_multipliers;
mod strategy;
mod strategies;

pub use risk_multipliers::{dd_multiplier, DdBin, VolBin, VolBinClassifier, VOL_BINS};
pub use strategies::*;
pub use strategy::{Strategy, StrategyContext, StrategyDecision, StrategyError, StrategyOrderType};

pub mod registry;
