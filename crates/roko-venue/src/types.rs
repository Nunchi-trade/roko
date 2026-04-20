//! Shared types used across all venue implementations.
//!
//! Deliberately venue-agnostic; HL- or Nunchi-specific fields live in the
//! per-backend modules under [`crate::hl`] / [`crate::nunchi`].

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Buy/sell direction. Stored as an enum rather than a string so the compiler
/// rejects typos at call sites. The Python stack used the strings "buy" and
/// "sell"; use [`OrderSide::as_str`] when speaking to that API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OrderSide {
    Buy,
    Sell,
}

impl OrderSide {
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Buy => "buy",
            Self::Sell => "sell",
        }
    }

    #[must_use]
    pub const fn is_long(self) -> bool {
        matches!(self, Self::Buy)
    }
}

/// Time-in-force policy on a limit order. Mirrors Hyperliquid's `tif` string.
///
/// Note: Nunchi's orderbook has no native ALO/trigger semantics today.
/// [`crate::nunchi::NunchiVenue`] treats `Alo` as a regular limit with a
/// warning, per the Python `NunchiVenueAdapter`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TimeInForce {
    /// Good-til-cancelled — rest on the book.
    Gtc,
    /// Immediate-or-cancel — cross the spread.
    Ioc,
    /// Add-liquidity-only — maker only.
    Alo,
}

impl TimeInForce {
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Gtc => "Gtc",
            Self::Ioc => "Ioc",
            Self::Alo => "Alo",
        }
    }
}

impl Default for TimeInForce {
    fn default() -> Self {
        Self::Ioc
    }
}

/// A venue market symbol. Canonical form is the HL-style human string
/// (`"ETH-PERP"`, `"VXX-USDYP"`). Backends translate to any internal id
/// (e.g. Nunchi's `bytes32 marketId`) behind the trait.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Instrument(pub String);

impl Instrument {
    #[must_use]
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for Instrument {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

impl std::fmt::Display for Instrument {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// Point-in-time market state returned by [`crate::VenueAdapter::get_snapshot`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MarketSnapshot {
    pub instrument: String,
    pub mid_price: f64,
    pub bid: f64,
    pub ask: f64,
    pub spread_bps: f64,
    pub timestamp_ms: i64,
    pub volume_24h: f64,
    pub funding_rate: f64,
    pub open_interest: f64,
}

/// A venue-agnostic trade fill. Mirrors the Python `common.venue_adapter.Fill`
/// dataclass. Prices and quantities are floats to match the strategy layer;
/// backends do fixed-point conversion internally before signing on-chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fill {
    pub oid: String,
    pub instrument: String,
    pub side: OrderSide,
    pub price: f64,
    pub quantity: f64,
    pub timestamp_ms: i64,
    pub fee: f64,
}

/// What optional capabilities a venue supports. Strategies check these before
/// calling optional methods on the trait.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct VenueCapabilities {
    pub supports_alo: bool,
    pub supports_trigger_orders: bool,
    pub supports_builder_fee: bool,
    pub supports_cross_margin: bool,
}

/// Error surface for venue operations. Backends map their native errors into
/// one of these variants so strategy code stays venue-agnostic.
#[derive(Debug, Error)]
pub enum VenueError {
    #[error("not connected — call VenueAdapter::connect() before market data / execution calls")]
    NotConnected,
    #[error("unknown instrument: {0}")]
    UnknownInstrument(String),
    #[error("rejected by venue: {0}")]
    Rejected(String),
    #[error("rpc / network error: {0}")]
    Network(String),
    #[error("signing error: {0}")]
    Signing(String),
    #[error("capability not supported by this venue")]
    Unsupported,
    #[error("other: {0}")]
    Other(String),
}
