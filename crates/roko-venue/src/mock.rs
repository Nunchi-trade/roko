//! Deterministic in-memory venue for tests.
//!
//! Matches the behavior of `adapters/mock_adapter.py` / `DirectMockProxy`
//! from `Nunchi-trade/offchainservices-agent`: synthetic snapshots, auto-
//! filled orders, no network.

use async_trait::async_trait;
use parking_lot_stub::Mutex;
use serde_json::{json, Value as JsonValue};
use std::collections::HashMap;

use crate::types::{Fill, Instrument, MarketSnapshot, OrderSide, TimeInForce, VenueCapabilities, VenueError};
use crate::venue::VenueAdapter;

mod parking_lot_stub {
    //! Minimal `Mutex` wrapper — we don't pull `parking_lot` into the venue
    //! crate's default deps just for the mock. `std::sync::Mutex` is fine
    //! here because contention is zero in practice (tests single-thread
    //! the mock) and we panic on poison anyway.
    pub struct Mutex<T>(std::sync::Mutex<T>);
    impl<T> Mutex<T> {
        pub const fn new(v: T) -> Self {
            Self(std::sync::Mutex::new(v))
        }
        pub fn lock(&self) -> std::sync::MutexGuard<'_, T> {
            self.0.lock().expect("mock venue mutex poisoned")
        }
    }
}

#[derive(Default)]
struct State {
    next_oid: u64,
    fills: Vec<Fill>,
    leverage: HashMap<String, (u32, bool)>,
}

/// A deterministic mock venue. Returns synthetic mid prices driven by the
/// configured `mid`, auto-fills any order at the submitted price.
pub struct MockVenue {
    mid: f64,
    state: Mutex<State>,
    connected: Mutex<bool>,
}

impl MockVenue {
    #[must_use]
    pub fn new() -> Self {
        Self {
            mid: 2500.0,
            state: Mutex::new(State::default()),
            connected: Mutex::new(false),
        }
    }

    #[must_use]
    pub fn with_mid(mid: f64) -> Self {
        Self {
            mid,
            state: Mutex::new(State::default()),
            connected: Mutex::new(false),
        }
    }
}

impl Default for MockVenue {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl VenueAdapter for MockVenue {
    async fn connect(&mut self, _private_key: &str, _testnet: bool) -> Result<(), VenueError> {
        *self.connected.lock() = true;
        Ok(())
    }

    fn capabilities(&self) -> VenueCapabilities {
        VenueCapabilities {
            supports_alo: true,
            supports_trigger_orders: true,
            supports_builder_fee: true,
            supports_cross_margin: true,
        }
    }

    async fn get_snapshot(&self, instrument: &Instrument) -> Result<MarketSnapshot, VenueError> {
        let bid = self.mid - 0.5;
        let ask = self.mid + 0.5;
        Ok(MarketSnapshot {
            instrument: instrument.as_str().to_string(),
            mid_price: self.mid,
            bid,
            ask,
            spread_bps: ((ask - bid) / self.mid) * 10_000.0,
            timestamp_ms: 0,
            ..Default::default()
        })
    }

    async fn get_candles(&self, _coin: &str, _interval: &str, _lookback_ms: i64) -> Result<Vec<JsonValue>, VenueError> {
        Ok(vec![])
    }

    async fn get_all_markets(&self) -> Result<Vec<Instrument>, VenueError> {
        Ok(vec![Instrument::new("ETH-PERP")])
    }

    async fn get_all_mids(&self) -> Result<HashMap<String, String>, VenueError> {
        let mut m = HashMap::new();
        m.insert("ETH-PERP".into(), self.mid.to_string());
        Ok(m)
    }

    async fn place_order(
        &self,
        instrument: &Instrument,
        side: OrderSide,
        size: f64,
        price: f64,
        _tif: TimeInForce,
        _builder: Option<JsonValue>,
    ) -> Result<Option<Fill>, VenueError> {
        let mut st = self.state.lock();
        st.next_oid += 1;
        let fill = Fill {
            oid: st.next_oid.to_string(),
            instrument: instrument.as_str().to_string(),
            side,
            price,
            quantity: size,
            timestamp_ms: 0,
            fee: 0.0,
        };
        st.fills.push(fill.clone());
        Ok(Some(fill))
    }

    async fn cancel_order(&self, _instrument: &Instrument, _oid: &str) -> Result<bool, VenueError> {
        Ok(true)
    }

    async fn get_open_orders(&self, _instrument: Option<&Instrument>) -> Result<Vec<JsonValue>, VenueError> {
        Ok(vec![])
    }

    async fn get_account_state(&self) -> Result<JsonValue, VenueError> {
        Ok(json!({
            "available_balance": 10_000.0,
            "cross_equity": 10_000.0,
            "positions": [],
        }))
    }

    async fn set_leverage(&self, leverage: u32, coin: &str, is_cross: bool) -> Result<(), VenueError> {
        self.state.lock().leverage.insert(coin.to_string(), (leverage, is_cross));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_connect_and_place() {
        let mut venue = MockVenue::new();
        venue.connect("0xdead", true).await.unwrap();
        let instrument = Instrument::new("ETH-PERP");
        let snap = venue.get_snapshot(&instrument).await.unwrap();
        assert!(snap.mid_price > 0.0);
        let fill = venue
            .place_order(&instrument, OrderSide::Buy, 0.01, 2500.5, TimeInForce::Ioc, None)
            .await
            .unwrap()
            .expect("mock always fills");
        assert_eq!(fill.instrument, "ETH-PERP");
        assert_eq!(fill.side, OrderSide::Buy);
    }

    #[tokio::test]
    async fn set_leverage_is_recorded() {
        let venue = MockVenue::new();
        venue.set_leverage(5, "ETH", false).await.unwrap();
        assert_eq!(
            venue.state.lock().leverage.get("ETH").copied(),
            Some((5, false))
        );
    }

    #[test]
    fn order_side_string_matches_python_api() {
        assert_eq!(OrderSide::Buy.as_str(), "buy");
        assert_eq!(OrderSide::Sell.as_str(), "sell");
    }

    #[test]
    fn tif_string_matches_hl_api() {
        assert_eq!(TimeInForce::Gtc.as_str(), "Gtc");
        assert_eq!(TimeInForce::Ioc.as_str(), "Ioc");
        assert_eq!(TimeInForce::Alo.as_str(), "Alo");
    }
}
