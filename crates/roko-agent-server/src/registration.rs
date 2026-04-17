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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
        state: &AgentState,
        addr: SocketAddr,
    ) -> Result<RegistrationOutcome> {
        let card = self.build_card(state, addr);
        let card_uri = self.publisher.publish(&card).await?;
        let tx_hash = if let (Some(wallet), Some(registry), Some(passport_id)) = (
            self.wallet.as_ref(),
            self.identity_registry_address.as_ref(),
            self.passport_id.as_ref(),
        ) {
            let request = TxRequest {
                to: Some(registry.clone()),
                data: build_update_agent_card_uri_calldata(passport_id, &card_uri),
                ..TxRequest::default()
            };
            let hash = wallet
                .sign_and_submit(request)
                .await
                .context("submit updateAgentCardUri transaction")?;
            Some(tx_hash_string(&hash))
        } else {
            None
        };

        let outcome = RegistrationOutcome {
            card,
            card_uri,
            tx_hash: tx_hash.clone(),
        };

        if tx_hash.is_none() {
            if let Some(callback) = &self.discovery_callback {
                callback(outcome.clone()).await?;
            }
        }

        Ok(outcome)
    }
}

fn tx_hash_string(hash: &TxHash) -> String {
    hash.as_str().to_string()
}

fn build_update_agent_card_uri_calldata(passport_id: &str, card_uri: &str) -> Vec<u8> {
    let selector = &keccak256("updateAgentCardUri(uint256,string)".as_bytes())[..4];
    let encoded_passport = encode_passport_id_word(passport_id);
    let encoded_card_uri = abi_encode_string(card_uri);
    let mut data = Vec::with_capacity(4 + 64 + encoded_card_uri.len());
    data.extend_from_slice(selector);
    data.extend_from_slice(&encoded_passport);
    data.extend_from_slice(&encode_word(64));
    data.extend_from_slice(&encoded_card_uri);
    data
}

fn encode_passport_id_word(passport_id: &str) -> [u8; 32] {
    let trimmed = passport_id.trim();
    let value = if let Some(hex) = trimmed.strip_prefix("0x") {
        U256::from_str_radix(hex, 16).expect("passport_id should be a valid hex uint256")
    } else {
        U256::from_str_radix(trimmed, 10).expect("passport_id should be a valid decimal uint256")
    };
    value.to_be_bytes::<32>()
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

    #[test]
    fn calldata_contains_selector_and_dynamic_offsets() {
        let calldata = build_update_agent_card_uri_calldata("7", "https://card");
        assert_eq!(
            &calldata[..4],
            &keccak256("updateAgentCardUri(uint256,string)".as_bytes())[..4]
        );
        assert_eq!(&calldata[4..36], &U256::from(7_u64).to_be_bytes::<32>());
        assert_eq!(&calldata[36..68], &encode_word(64));
        assert_eq!(calldata.len() % 32, 4);
    }
}
