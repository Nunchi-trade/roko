//! Venue-agnostic trading interface for the roko trading stack.
//!
//! Mirrors `Nunchi-trade/offchainservices-agent`'s `common/venue_adapter.py`:
//! strategies depend on [`VenueAdapter`] + [`Fill`] and nothing venue-specific.
//! Concrete impls:
//!
//! - [`hl::HyperliquidVenue`] (feature `hl-backend`) — Hyperliquid perps +
//!   YEX via `hyperliquid-rust-sdk`.
//! - [`nunchi::NunchiVenue`] (feature `nunchi-backend`) — on-chain Nunchi
//!   orderbook (contracts-core / `packages/exchange`), driven through
//!   `roko-chain`'s alloy-backed `ChainClient` / `ChainWallet`.
//! - [`mock::MockVenue`] — deterministic in-memory venue for tests.
//!
//! See `plans/P08-trading-surface.md` (T2) for the tasks port-plan.

#![allow(missing_docs)]
#![forbid(unsafe_code)]

pub mod mock;
#[cfg(feature = "nunchi-backend")]
pub mod nunchi;
#[cfg(feature = "hl-backend")]
pub mod hl;

mod types;
mod venue;

pub use types::{Fill, Instrument, MarketSnapshot, OrderSide, TimeInForce, VenueCapabilities, VenueError};
pub use venue::VenueAdapter;
