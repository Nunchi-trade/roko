//! The [`VenueAdapter`] trait that all venue backends implement.

use async_trait::async_trait;
use serde_json::Value as JsonValue;

use crate::types::{Fill, Instrument, MarketSnapshot, OrderSide, TimeInForce, VenueCapabilities, VenueError};

/// Venue-agnostic trading + market-data interface.
///
/// Ports the Python `common.venue_adapter.VenueAdapter` ABC from
/// `Nunchi-trade/offchainservices-agent`. Strategies use only this trait —
/// no direct dependency on Hyperliquid or Nunchi APIs.
///
/// Construction and credential binding are deliberately split: implementors
/// are built cheaply, then [`VenueAdapter::connect`] performs any I/O or
/// keystore lookup. This mirrors the Python `connect()` contract and keeps
/// tests from needing real credentials at construction time.
#[async_trait]
pub trait VenueAdapter: Send + Sync {
    /// Bind credentials. `testnet` is honored by backends that distinguish
    /// chains (Hyperliquid does); Nunchi ignores it because the network is
    /// set at construction time via its manifest.
    async fn connect(&mut self, private_key: &str, testnet: bool) -> Result<(), VenueError>;

    /// Optional-feature matrix — strategies check this before calling the
    /// optional methods at the bottom of the trait.
    fn capabilities(&self) -> VenueCapabilities;

    // ---------------------------------------------------------------- market data

    async fn get_snapshot(&self, instrument: &Instrument) -> Result<MarketSnapshot, VenueError>;

    /// Return OHLC candles. Nunchi returns `[]` today (no native OHLC);
    /// strategies that need candles can feature-detect via an empty slice.
    async fn get_candles(
        &self,
        coin: &str,
        interval: &str,
        lookback_ms: i64,
    ) -> Result<Vec<JsonValue>, VenueError>;

    async fn get_all_markets(&self) -> Result<Vec<Instrument>, VenueError>;

    async fn get_all_mids(&self) -> Result<std::collections::HashMap<String, String>, VenueError>;

    // ---------------------------------------------------------------- execution

    /// Place a single order. Returns `Ok(Some(fill))` on match, `Ok(None)` if
    /// the venue accepted a resting order or rejected silently.
    async fn place_order(
        &self,
        instrument: &Instrument,
        side: OrderSide,
        size: f64,
        price: f64,
        tif: TimeInForce,
        builder: Option<JsonValue>,
    ) -> Result<Option<Fill>, VenueError>;

    async fn cancel_order(&self, instrument: &Instrument, oid: &str) -> Result<bool, VenueError>;

    async fn get_open_orders(&self, instrument: Option<&Instrument>) -> Result<Vec<JsonValue>, VenueError>;

    // ---------------------------------------------------------------- account

    async fn get_account_state(&self) -> Result<JsonValue, VenueError>;

    /// Update leverage for an instrument. Semantics differ per venue —
    /// Hyperliquid does an on-chain set, Nunchi stores an in-memory pref
    /// applied at `place_order` time.
    async fn set_leverage(&self, leverage: u32, coin: &str, is_cross: bool) -> Result<(), VenueError>;

    // ---------------------------------------------------------------- optional

    async fn place_trigger_order(
        &self,
        _instrument: &Instrument,
        _side: OrderSide,
        _size: f64,
        _trigger_price: f64,
        _builder: Option<JsonValue>,
    ) -> Result<Option<String>, VenueError> {
        Err(VenueError::Unsupported)
    }

    async fn cancel_trigger_order(&self, _instrument: &Instrument, _oid: &str) -> Result<bool, VenueError> {
        Err(VenueError::Unsupported)
    }
}
