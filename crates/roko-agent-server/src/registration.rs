//! ERC-8004 agent-card publishing and best-effort identity-registry updates.

use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;

use alloy_primitives::{U256, keccak256};
use anyhow::{Context, Result};
use async_trait::async_trait;
use base64::Engine;
use roko_chain::{ChainWallet, TxHash, TxRequest};
use serde::{Deserialize, Serialize};

use crate::features::relay_client::{self, RelayClientConfig};
use crate::state::AgentState;

type BoxFutureResult = Pin<Box<dyn Future<Output = Result<()>> + Send>>;

/// ERC-8004 Agent Card payload.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentCard {
    /// Human-readable agent name.
    pub name: String,
    /// Advertised capabilities.
    pub capabilities: Vec<String>,
    /// Endpoint map used for discovery.
    pub endpoints: AgentCardEndpoints,
    /// Domain tags used for off-chain filtering.
    pub domain_tags: Vec<String>,
    /// Card schema/version.
    pub version: String,
}

/// Agent-card endpoint fields.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct AgentCardEndpoints {
    /// REST endpoint.
    pub rest: Option<String>,
    /// WebSocket endpoint.
    pub websocket: Option<String>,
    /// A2A endpoint.
    pub a2a: Option<String>,
    /// MCP endpoint.
    pub mcp: Option<String>,
}

/// Result of an agent-card publication/update.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegistrationOutcome {
    /// Published card.
    pub card: AgentCard,
    /// Published URI.
    pub card_uri: String,
    /// Submitted transaction hash when on-chain registration succeeded.
    pub tx_hash: Option<String>,
}

/// Publish a card JSON payload and return its URI.
#[async_trait]
pub trait AgentCardPublisher: Send + Sync {
    /// Publish a card and return a stable URI.
    async fn publish(&self, card: &AgentCard) -> Result<String>;
}

/// Simple data-URI publisher used when no external publisher is provided.
#[derive(Debug, Default)]
pub struct DataUriPublisher;

#[async_trait]
impl AgentCardPublisher for DataUriPublisher {
    async fn publish(&self, card: &AgentCard) -> Result<String> {
        let json = serde_json::to_vec(card).context("serialize agent card")?;
        Ok(format!(
            "data:application/json;base64,{}",
            base64::engine::general_purpose::STANDARD_NO_PAD.encode(json)
        ))
    }
}

/// Registration configuration for best-effort agent-card publication.
#[derive(Clone)]
pub struct AgentRegistration {
    /// Optional external publisher.
    pub publisher: Arc<dyn AgentCardPublisher>,
    /// Optional outbound relay bridge configuration.
    pub relay: Option<RelayClientConfig>,
    /// Optional identity-registry contract address.
    pub identity_registry_address: Option<String>,
    /// Optional passport identifier used for `updateAgentCardUri`.
    pub passport_id: Option<String>,
    /// Optional signing wallet.
    pub wallet: Option<Arc<dyn ChainWallet>>,
    /// Optional callback for non-wallet discovery registration.
    pub discovery_callback:
        Option<Arc<dyn Fn(RegistrationOutcome) -> BoxFutureResult + Send + Sync>>,
    /// Extra domain tags to merge into the published card.
    pub domain_tags: Vec<String>,
}

impl std::fmt::Debug for AgentRegistration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentRegistration")
            .field("relay", &self.relay)
            .field("identity_registry_address", &self.identity_registry_address)
            .field("passport_id", &self.passport_id)
            .field("domain_tags", &self.domain_tags)
            .finish_non_exhaustive()
    }
}

impl Default for AgentRegistration {
    fn default() -> Self {
        Self {
            publisher: Arc::new(DataUriPublisher),
            relay: None,
            identity_registry_address: None,
            passport_id: None,
            wallet: None,
            discovery_callback: None,
            domain_tags: vec!["roko".to_string()],
        }
    }
}

impl AgentRegistration {
    /// Build an agent card for the currently bound address.
    #[must_use]
    pub fn build_card(&self, state: &AgentState, addr: SocketAddr) -> AgentCard {
        let mut card = state.build_agent_card(addr);
        for tag in &self.domain_tags {
            if !card.domain_tags.iter().any(|existing| existing == tag) {
                card.domain_tags.push(tag.clone());
            }
        }
        card
    }

    /// Publish the card and optionally submit an `updateAgentCardUri` transaction.
    ///
    /// # Errors
    ///
    /// Returns an error if the card cannot be published, if the optional
    /// on-chain transaction cannot be signed or submitted, or if the optional
    /// discovery callback fails.
    pub async fn register(
        &self,
        state: Arc<AgentState>,
        addr: SocketAddr,
    ) -> Result<RegistrationOutcome> {
        let mut outcome = self.publish_card(Arc::clone(&state), addr).await?;
        outcome.tx_hash = self.update_identity_registry(&outcome.card_uri).await?;

        if outcome.tx_hash.is_none() {
            self.publish_wallet_free_registration(outcome.clone())
                .await?;
        }

        Ok(outcome)
    }

    async fn publish_card(
        &self,
        state: Arc<AgentState>,
        addr: SocketAddr,
    ) -> Result<RegistrationOutcome> {
        let card = self.build_card(state.as_ref(), addr);
        let card_uri = if let Some(relay) = &self.relay {
            let card_uri = relay.card_uri(state.agent_id())?;
            relay_client::connect(relay.clone(), state, card.clone())
                .await
                .context("connect agent relay client")?;
            card_uri
        } else {
            self.publisher.publish(&card).await?
        };
        Ok(RegistrationOutcome {
            card,
            card_uri,
            tx_hash: None,
        })
    }

    async fn update_identity_registry(&self, card_uri: &str) -> Result<Option<String>> {
        let Some((wallet, registry, passport_id)) = self.chain_update_config() else {
            return Ok(None);
        };
        let request = TxRequest {
            to: Some(registry.to_string()),
            data: build_update_agent_card_uri_calldata(passport_id, card_uri)?,
            ..TxRequest::default()
        };
        let hash = wallet
            .sign_and_submit(request)
            .await
            .context("submit updateAgentCardUri transaction")?;
        Ok(Some(tx_hash_string(&hash)))
    }

    fn chain_update_config(&self) -> Option<(&dyn ChainWallet, &str, &str)> {
        Some((
            self.wallet.as_deref()?,
            self.identity_registry_address.as_deref()?,
            self.passport_id.as_deref()?,
        ))
    }

    async fn publish_wallet_free_registration(&self, outcome: RegistrationOutcome) -> Result<()> {
        if let Some(callback) = &self.discovery_callback {
            callback(outcome).await?;
        }
        Ok(())
    }
}

fn tx_hash_string(hash: &TxHash) -> String {
    hash.as_str().to_string()
}

fn build_update_agent_card_uri_calldata(passport_id: &str, card_uri: &str) -> Result<Vec<u8>> {
    let selector = &keccak256("updateAgentCardUri(uint256,string)".as_bytes())[..4];
    let encoded_passport = encode_passport_id_word(passport_id)?;
    let encoded_card_uri = abi_encode_string(card_uri);
    let mut data = Vec::with_capacity(4 + 64 + encoded_card_uri.len());
    data.extend_from_slice(selector);
    data.extend_from_slice(&encoded_passport);
    data.extend_from_slice(&encode_word(64));
    data.extend_from_slice(&encoded_card_uri);
    Ok(data)
}

fn encode_passport_id_word(passport_id: &str) -> Result<[u8; 32]> {
    let trimmed = passport_id.trim();
    let value = if let Some(hex) = trimmed.strip_prefix("0x") {
        U256::from_str_radix(hex, 16).context("parse passport_id as hex uint256")?
    } else {
        U256::from_str_radix(trimmed, 10).context("parse passport_id as decimal uint256")?
    };
    Ok(value.to_be_bytes::<32>())
}

fn abi_encode_string(value: &str) -> Vec<u8> {
    let bytes = value.as_bytes();
    let padded = bytes.len().next_multiple_of(32);
    let mut encoded = Vec::with_capacity(32 + padded);
    encoded.extend_from_slice(&encode_word(bytes.len() as u64));
    encoded.extend_from_slice(bytes);
    encoded.resize(32 + padded, 0);
    encoded
}

fn encode_word(value: u64) -> [u8; 32] {
    let mut out = [0_u8; 32];
    out[24..].copy_from_slice(&value.to_be_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Arc;

    use anyhow::anyhow;
    use parking_lot::Mutex;
    use roko_chain::{BlockNumber, ChainResult, Receipt};

    struct StubPublisher {
        uri: String,
        published_cards: Mutex<Vec<AgentCard>>,
    }

    impl StubPublisher {
        fn new(uri: impl Into<String>) -> Self {
            Self {
                uri: uri.into(),
                published_cards: Mutex::new(Vec::new()),
            }
        }

        fn published_cards(&self) -> Vec<AgentCard> {
            self.published_cards.lock().clone()
        }
    }

    #[async_trait]
    impl AgentCardPublisher for StubPublisher {
        async fn publish(&self, card: &AgentCard) -> Result<String> {
            self.published_cards.lock().push(card.clone());
            Ok(self.uri.clone())
        }
    }

    struct StubWallet {
        submitted: Mutex<Vec<TxRequest>>,
        tx_hash: TxHash,
    }

    impl StubWallet {
        fn new(tx_hash: impl Into<String>) -> Self {
            Self {
                submitted: Mutex::new(Vec::new()),
                tx_hash: TxHash::new(tx_hash.into()),
            }
        }

        fn submitted(&self) -> Vec<TxRequest> {
            self.submitted.lock().clone()
        }
    }

    #[async_trait]
    impl ChainWallet for StubWallet {
        async fn address(&self) -> ChainResult<String> {
            Ok("0x000000000000000000000000000000000000beef".to_string())
        }

        async fn balance(&self, _block: Option<BlockNumber>) -> ChainResult<u128> {
            Ok(0)
        }

        async fn nonce(&self) -> ChainResult<u64> {
            Ok(0)
        }

        async fn sign_and_submit(&self, tx: TxRequest) -> ChainResult<TxHash> {
            self.submitted.lock().push(tx);
            Ok(self.tx_hash.clone())
        }

        async fn wait_for_receipt(&self, tx: &TxHash, _timeout_ms: u64) -> ChainResult<Receipt> {
            Err(roko_chain::ChainError::Unsupported(format!(
                "wait_for_receipt not implemented for {tx}"
            )))
        }

        fn name(&self) -> &str {
            "stub-wallet"
        }
    }

    fn test_state() -> AgentState {
        AgentState::new(
            "agent-1".to_string(),
            None,
            "1.0".to_string(),
            vec!["chat".to_string()],
            None,
            None,
            None,
        )
    }

    #[test]
    fn calldata_contains_selector_and_dynamic_offsets() {
        let calldata = build_update_agent_card_uri_calldata("7", "https://card").expect("calldata");
        assert_eq!(
            &calldata[..4],
            &keccak256("updateAgentCardUri(uint256,string)".as_bytes())[..4]
        );
        assert_eq!(&calldata[4..36], &U256::from(7_u64).to_be_bytes::<32>());
        assert_eq!(&calldata[36..68], &encode_word(64));
        assert_eq!(calldata.len() % 32, 4);
    }

    #[test]
    fn calldata_accepts_hex_passport_ids() {
        let calldata =
            build_update_agent_card_uri_calldata("0x2a", "https://card").expect("calldata");
        assert_eq!(&calldata[4..36], &U256::from(42_u64).to_be_bytes::<32>());
    }

    #[test]
    fn invalid_passport_id_returns_error() {
        let error =
            build_update_agent_card_uri_calldata("not-a-uint", "https://card").expect_err("error");
        assert!(error.to_string().contains("parse passport_id"));
    }

    #[tokio::test]
    async fn wallet_free_registration_publishes_and_calls_discovery_callback() {
        let publisher = Arc::new(StubPublisher::new("https://relay.example/card.json"));
        let callback_outcomes = Arc::new(Mutex::new(Vec::new()));
        let callback_outcomes_clone = Arc::clone(&callback_outcomes);
        let registration = AgentRegistration {
            publisher: publisher.clone(),
            discovery_callback: Some(Arc::new(move |outcome| {
                let callback_outcomes = Arc::clone(&callback_outcomes_clone);
                Box::pin(async move {
                    callback_outcomes.lock().push(outcome);
                    Ok(())
                })
            })),
            domain_tags: vec!["relay".to_string()],
            ..AgentRegistration::default()
        };

        let outcome = registration
            .register(
                Arc::new(test_state()),
                "127.0.0.1:8080".parse().expect("addr"),
            )
            .await
            .expect("register");

        assert_eq!(outcome.tx_hash, None);
        assert_eq!(outcome.card_uri, "https://relay.example/card.json");
        assert!(outcome.card.domain_tags.iter().any(|tag| tag == "roko"));
        assert!(outcome.card.domain_tags.iter().any(|tag| tag == "relay"));
        assert_eq!(publisher.published_cards(), vec![outcome.card.clone()]);
        assert_eq!(callback_outcomes.lock().clone(), vec![outcome]);
    }

    #[tokio::test]
    async fn on_chain_registration_submits_update_and_skips_discovery_callback() {
        let publisher = Arc::new(StubPublisher::new("https://relay.example/card.json"));
        let wallet = Arc::new(StubWallet::new("0xfeed"));
        let callback_count = Arc::new(Mutex::new(0_usize));
        let callback_count_clone = Arc::clone(&callback_count);
        let registration = AgentRegistration {
            publisher,
            wallet: Some(wallet.clone()),
            identity_registry_address: Some(
                "0x000000000000000000000000000000000000c0de".to_string(),
            ),
            passport_id: Some("7".to_string()),
            discovery_callback: Some(Arc::new(move |_outcome| {
                let callback_count = Arc::clone(&callback_count_clone);
                Box::pin(async move {
                    *callback_count.lock() += 1;
                    Err(anyhow!(
                        "wallet-backed registration should not use discovery callback"
                    ))
                })
            })),
            ..AgentRegistration::default()
        };

        let outcome = registration
            .register(
                Arc::new(test_state()),
                "127.0.0.1:8080".parse().expect("addr"),
            )
            .await
            .expect("register");

        assert_eq!(outcome.tx_hash.as_deref(), Some("0xfeed"));
        assert_eq!(*callback_count.lock(), 0);

        let submitted = wallet.submitted();
        assert_eq!(submitted.len(), 1);
        assert_eq!(
            submitted[0].to.as_deref(),
            Some("0x000000000000000000000000000000000000c0de")
        );
        assert_eq!(
            submitted[0].data,
            build_update_agent_card_uri_calldata("7", "https://relay.example/card.json")
                .expect("calldata")
        );
    }

    #[tokio::test]
    async fn missing_wallet_keeps_chain_update_optional() {
        let callback_count = Arc::new(Mutex::new(0_usize));
        let callback_count_clone = Arc::clone(&callback_count);
        let registration = AgentRegistration {
            publisher: Arc::new(StubPublisher::new("https://relay.example/card.json")),
            identity_registry_address: Some(
                "0x000000000000000000000000000000000000c0de".to_string(),
            ),
            passport_id: Some("7".to_string()),
            discovery_callback: Some(Arc::new(move |_outcome| {
                let callback_count = Arc::clone(&callback_count_clone);
                Box::pin(async move {
                    *callback_count.lock() += 1;
                    Ok(())
                })
            })),
            ..AgentRegistration::default()
        };

        let outcome = registration
            .register(
                Arc::new(test_state()),
                "127.0.0.1:8080".parse().expect("addr"),
            )
            .await
            .expect("register");

        assert_eq!(outcome.tx_hash, None);
        assert_eq!(*callback_count.lock(), 1);
    }
}
