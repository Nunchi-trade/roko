//! Hyperliquid venue, feature-gated behind `hl-backend`.
//!
//! Placeholder for the port of `parent/hl_proxy.py` + `cli/hl_adapter.py`
//! from `Nunchi-trade/offchainservices-agent`. Lands in a follow-up commit
//! (plans/P08-trading-surface.md T2 close-out).

use async_trait::async_trait;
use serde_json::Value as JsonValue;
use std::collections::HashMap;

use crate::types::{Fill, Instrument, MarketSnapshot, OrderSide, TimeInForce, VenueCapabilities, VenueError};
use crate::venue::VenueAdapter;

/// Hyperliquid perps + YEX venue.
///
/// Placeholder impl — compiles but every method returns [`VenueError::Unsupported`]
/// until the hyperliquid-rust-sdk wire-up lands.
pub struct HyperliquidVenue {
    testnet: bool,
}

impl HyperliquidVenue {
    #[must_use]
    pub fn new() -> Self {
        Self { testnet: true }
    }
}

impl Default for HyperliquidVenue {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl VenueAdapter for HyperliquidVenue {
    async fn connect(&mut self, _private_key: &str, testnet: bool) -> Result<(), VenueError> {
        self.testnet = testnet;
        Err(VenueError::Unsupported)
    }

    fn capabilities(&self) -> VenueCapabilities {
        VenueCapabilities {
            supports_alo: true,
            supports_trigger_orders: true,
            supports_builder_fee: true,
            supports_cross_margin: true,
        }
    }

    async fn get_snapshot(&self, _i: &Instrument) -> Result<MarketSnapshot, VenueError> {
        Err(VenueError::Unsupported)
    }

    async fn get_candles(&self, _c: &str, _i: &str, _l: i64) -> Result<Vec<JsonValue>, VenueError> {
        Err(VenueError::Unsupported)
    }

    async fn get_all_markets(&self) -> Result<Vec<Instrument>, VenueError> {
        Err(VenueError::Unsupported)
    }

    async fn get_all_mids(&self) -> Result<HashMap<String, String>, VenueError> {
        Err(VenueError::Unsupported)
    }

    async fn place_order(
        &self,
        _i: &Instrument,
        _s: OrderSide,
        _sz: f64,
        _p: f64,
        _t: TimeInForce,
        _b: Option<JsonValue>,
    ) -> Result<Option<Fill>, VenueError> {
        Err(VenueError::Unsupported)
    }

    async fn cancel_order(&self, _i: &Instrument, _o: &str) -> Result<bool, VenueError> {
        Err(VenueError::Unsupported)
    }

    async fn get_open_orders(&self, _i: Option<&Instrument>) -> Result<Vec<JsonValue>, VenueError> {
        Err(VenueError::Unsupported)
    }

    async fn get_account_state(&self) -> Result<JsonValue, VenueError> {
        Err(VenueError::Unsupported)
    }

    async fn set_leverage(&self, _l: u32, _c: &str, _x: bool) -> Result<(), VenueError> {
        Err(VenueError::Unsupported)
    }
}
