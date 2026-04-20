//! Nunchi on-chain orderbook venue, feature-gated behind `nunchi-backend`.
//!
//! Mirrors `Nunchi-trade/offchainservices-agent`'s Python `NunchiVenueAdapter`
//! but expressed in Rust using `roko-chain`'s alloy-backed chain client.
//!
//! This module deliberately keeps the low-level RPC wiring small — we only
//! expose what strategies actually need. The full ABI binding set lands in a
//! follow-up commit (see plans/P08-trading-surface.md T2 close-out) once
//! devnet1 addresses are published and we can exercise the path end-to-end.

use async_trait::async_trait;
use serde_json::Value as JsonValue;
use std::collections::HashMap;

use crate::types::{Fill, Instrument, MarketSnapshot, OrderSide, TimeInForce, VenueCapabilities, VenueError};
use crate::venue::VenueAdapter;

/// Nunchi contract addresses for a single named network. Mirrors
/// `adapters/nunchi_addresses.py::NunchiAddresses` in the Python stack.
#[derive(Debug, Clone)]
pub struct NunchiAddresses {
    pub chain_id: u64,
    pub rpc_url: Option<String>,
    pub clearing_house: Option<String>,
    pub market_registry: Option<String>,
    pub account_nft: Option<String>,
    pub pyth: Option<String>,
    pub stork: Option<String>,
    pub orderbooks: HashMap<String, String>,
}

impl NunchiAddresses {
    #[must_use]
    pub fn is_ready(&self) -> bool {
        self.clearing_house.is_some() && self.market_registry.is_some() && self.account_nft.is_some()
    }
}

/// Nunchi on-chain orderbook venue.
///
/// Construction takes pinned addresses + an account id. [`VenueAdapter::connect`]
/// binds the trading-agent private key (the EOA previously authorized via
/// `ClearingHouse.setTradingAgent(accountId, agent)`).
pub struct NunchiVenue {
    addresses: NunchiAddresses,
    account_id: u64,
    private_key: Option<String>,
}

impl NunchiVenue {
    #[must_use]
    pub fn new(addresses: NunchiAddresses, account_id: u64) -> Self {
        Self {
            addresses,
            account_id,
            private_key: None,
        }
    }

    /// Account id bound at construction. Matches the ERC-721 tokenId.
    #[must_use]
    pub fn account_id(&self) -> u64 {
        self.account_id
    }
}

#[async_trait]
impl VenueAdapter for NunchiVenue {
    async fn connect(&mut self, private_key: &str, _testnet: bool) -> Result<(), VenueError> {
        if !self.addresses.is_ready() {
            return Err(VenueError::Other(
                "NunchiAddresses is missing clearing_house / market_registry / account_nft; \
                 populate the manifest before calling connect()"
                    .into(),
            ));
        }
        if self.addresses.rpc_url.is_none() {
            return Err(VenueError::Other(
                "NunchiAddresses.rpc_url is None; pass --rpc-url or populate the manifest".into(),
            ));
        }
        self.private_key = Some(private_key.to_string());
        Ok(())
    }

    fn capabilities(&self) -> VenueCapabilities {
        VenueCapabilities {
            supports_alo: false,
            supports_trigger_orders: false,
            supports_builder_fee: false,
            supports_cross_margin: true,
        }
    }

    // Read path — stubbed until T2 close-out wires in roko-chain alloy calls.
    // Strategies running on NunchiVenue in this commit hit NotConnected-style
    // errors which is the explicit signal to populate the manifest.

    async fn get_snapshot(&self, _instrument: &Instrument) -> Result<MarketSnapshot, VenueError> {
        self.require_private_key()?;
        Err(VenueError::Other(
            "NunchiVenue::get_snapshot: alloy RPC path lands in P08/T2 close-out".into(),
        ))
    }

    async fn get_candles(&self, _coin: &str, _interval: &str, _lookback_ms: i64) -> Result<Vec<JsonValue>, VenueError> {
        // Nunchi emits no native OHLC — permanent `[]` (see plan P08).
        Ok(vec![])
    }

    async fn get_all_markets(&self) -> Result<Vec<Instrument>, VenueError> {
        let mut markets: Vec<Instrument> = self.addresses.orderbooks.keys().cloned().map(Instrument::new).collect();
        markets.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        Ok(markets)
    }

    async fn get_all_mids(&self) -> Result<HashMap<String, String>, VenueError> {
        // Same as get_snapshot — wire up via alloy in T2 close-out.
        self.require_private_key()?;
        Ok(HashMap::new())
    }

    async fn place_order(
        &self,
        _instrument: &Instrument,
        _side: OrderSide,
        _size: f64,
        _price: f64,
        _tif: TimeInForce,
        _builder: Option<JsonValue>,
    ) -> Result<Option<Fill>, VenueError> {
        self.require_private_key()?;
        Err(VenueError::Other(
            "NunchiVenue::place_order: on-chain tx path lands in P08/T2 close-out".into(),
        ))
    }

    async fn cancel_order(&self, _instrument: &Instrument, _oid: &str) -> Result<bool, VenueError> {
        self.require_private_key()?;
        Err(VenueError::Other(
            "NunchiVenue::cancel_order: on-chain tx path lands in P08/T2 close-out".into(),
        ))
    }

    async fn get_open_orders(&self, _instrument: Option<&Instrument>) -> Result<Vec<JsonValue>, VenueError> {
        self.require_private_key()?;
        Ok(vec![])
    }

    async fn get_account_state(&self) -> Result<JsonValue, VenueError> {
        self.require_private_key()?;
        Ok(serde_json::json!({
            "account_id": self.account_id,
            "ready": false,
            "note": "NunchiVenue live account-state wire-up lands in P08/T2 close-out",
        }))
    }

    async fn set_leverage(&self, _leverage: u32, _coin: &str, _is_cross: bool) -> Result<(), VenueError> {
        // Nunchi leverage is a per-order field; we'd cache here in T2 close-out.
        Ok(())
    }
}

impl NunchiVenue {
    fn require_private_key(&self) -> Result<(), VenueError> {
        if self.private_key.is_none() {
            Err(VenueError::NotConnected)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_addrs() -> NunchiAddresses {
        NunchiAddresses {
            chain_id: 0,
            rpc_url: None,
            clearing_house: None,
            market_registry: None,
            account_nft: None,
            pyth: None,
            stork: None,
            orderbooks: HashMap::new(),
        }
    }

    fn populated_addrs() -> NunchiAddresses {
        let mut orderbooks = HashMap::new();
        orderbooks.insert("ETH-PERP".into(), "0xaaaa".into());
        orderbooks.insert("BTC-PERP".into(), "0xbbbb".into());
        NunchiAddresses {
            chain_id: 31337,
            rpc_url: Some("http://127.0.0.1:8545".into()),
            clearing_house: Some("0x1111".into()),
            market_registry: Some("0x2222".into()),
            account_nft: Some("0x3333".into()),
            pyth: None,
            stork: None,
            orderbooks,
        }
    }

    #[tokio::test]
    async fn connect_refuses_incomplete_manifest() {
        let mut venue = NunchiVenue::new(empty_addrs(), 1);
        let err = venue.connect("0xdead", true).await.unwrap_err();
        assert!(matches!(err, VenueError::Other(_)));
    }

    #[tokio::test]
    async fn connect_requires_rpc_url() {
        let mut addrs = populated_addrs();
        addrs.rpc_url = None;
        let mut venue = NunchiVenue::new(addrs, 1);
        let err = venue.connect("0xdead", true).await.unwrap_err();
        assert!(matches!(err, VenueError::Other(_)));
    }

    #[tokio::test]
    async fn connect_happy_path() {
        let mut venue = NunchiVenue::new(populated_addrs(), 7);
        venue.connect("0xdeadbeefcafebabe", false).await.unwrap();
        assert_eq!(venue.account_id(), 7);
    }

    #[tokio::test]
    async fn get_all_markets_reads_manifest() {
        let venue = NunchiVenue::new(populated_addrs(), 7);
        let markets = venue.get_all_markets().await.unwrap();
        assert_eq!(
            markets.into_iter().map(|i| i.as_str().to_string()).collect::<Vec<_>>(),
            vec!["BTC-PERP".to_string(), "ETH-PERP".into()]
        );
    }

    #[tokio::test]
    async fn get_candles_is_permanent_empty() {
        let venue = NunchiVenue::new(populated_addrs(), 7);
        assert!(venue.get_candles("ETH", "1h", 3_600_000).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn capabilities_match_python_adapter() {
        let venue = NunchiVenue::new(populated_addrs(), 7);
        let caps = venue.capabilities();
        assert!(!caps.supports_alo);
        assert!(!caps.supports_trigger_orders);
        assert!(!caps.supports_builder_fee);
        assert!(caps.supports_cross_margin);
    }

    #[tokio::test]
    async fn ops_before_connect_fail() {
        let venue = NunchiVenue::new(populated_addrs(), 7);
        let err = venue.get_snapshot(&Instrument::new("ETH-PERP")).await.unwrap_err();
        assert!(matches!(err, VenueError::NotConnected));
    }
}
