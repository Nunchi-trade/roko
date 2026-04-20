//! Concrete strategy implementations. See `plans/P08-trading-surface.md` T3.

mod aggressive_taker;
mod avellaneda_mm;
mod basis_arb;
mod funding_momentum;
mod grid_mm;
mod hedge_agent;
mod mean_reversion;
mod momentum_breakout;
mod oi_divergence;
mod simple_mm;
mod trend_follower;

pub use aggressive_taker::AggressiveTakerStrategy;
pub use avellaneda_mm::AvellanedaStoikovStrategy;
pub use basis_arb::BasisArbStrategy;
pub use funding_momentum::FundingMomentumStrategy;
pub use grid_mm::GridMMStrategy;
pub use hedge_agent::HedgeAgent;
pub use mean_reversion::MeanReversionStrategy;
pub use momentum_breakout::MomentumBreakoutStrategy;
pub use oi_divergence::OIDivergenceStrategy;
pub use simple_mm::SimpleMMStrategy;
pub use trend_follower::TrendFollowerStrategy;

// Engine-backed strategies (funding_arb, liquidation_mm, engine_mm, regime_mm,
// simplified_ensemble) depend on roko-quoting — they land in the T4 close-out
// commit. LLM-backed strategies (claude_agent, rfq_agent) bind to a
// `roko-agent::Agent` backend via roko-trading-agent and land with T10.
