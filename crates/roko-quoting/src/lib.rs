//! Wave-based quoting engine — ports `offchainservices-agent/quoting_engine/`.
//!
//! The top-level [`QuotingEngine`] is a stateful per-market component that
//! produces a ladder of bid/ask quotes given the current mid/bid/ask,
//! inventory, drawdown, and optional external signals.
//!
//! See `plans/P08-trading-surface.md` T4.

#![forbid(unsafe_code)]
#![allow(missing_docs)]

mod config;
mod engine;
mod fair_value;
mod inventory;
mod ladder;
mod spread;
mod vol_estimator;

pub use config::{
    DisagreementConfig, FairValueBandConfig, FairValueWeights, FeedConfig, FundingBoundaryConfig,
    LadderParams, LiquidationDetectorConfig, MarketConfig, OracleMonitorConfig, RegimeOverride,
    SessionRegimeConfig, SkewParams, SpreadParams,
};
pub use engine::{QuoteResult, QuotingEngine, QuotingMeta};
pub use fair_value::FairValueCalculator;
pub use inventory::{InventorySkewer, InventoryState};
pub use ladder::{LadderBuilder, LadderLevel};
pub use spread::SpreadCalculator;
pub use vol_estimator::RollingVolEstimator;
