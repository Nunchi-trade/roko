//! Runtime chain context passed into fixtures + scenario spines.

use std::collections::HashMap;
use std::sync::Arc;

use alloy::network::EthereumWallet;
use alloy::primitives::Address;
use alloy::providers::{DynProvider, Provider, ProviderBuilder};
use alloy::signers::local::PrivateKeySigner;

use crate::manifest::Wallets;

/// Handy bundle: RPC url + chain id + signers + deployed addresses.
#[derive(Clone)]
pub struct ChainCtx {
    /// Endpoint URL.
    pub rpc_url: String,
    /// Chain id.
    pub chain_id: u64,
    /// Raw wallets (unsigned) — pass into contract instances as needed.
    pub wallets: Wallets,
    /// contract-name → 0x-address.
    pub addresses: HashMap<String, String>,
    /// Block at which the suite was deployed.
    pub deployed_at_block: u64,
}

impl ChainCtx {
    /// Build an alloy provider that signs as the named wallet.
    ///
    /// # Errors
    ///
    /// Returns an error if the RPC URL is invalid, the wallet is unknown, or
    /// the wallet private key cannot be parsed.
    pub fn wallet_provider(&self, wallet_name: &str) -> anyhow::Result<Arc<DynProvider>> {
        let entry = self
            .wallets
            .get(wallet_name)
            .ok_or_else(|| anyhow::anyhow!("unknown wallet: {wallet_name}"))?;
        let url = reqwest::Url::parse(&self.rpc_url)?;
        let signer: PrivateKeySigner = entry
            .private_key
            .trim_start_matches("0x")
            .parse()
            .map_err(|e| anyhow::anyhow!("parse key {wallet_name}: {e}"))?;
        let provider = ProviderBuilder::new()
            .wallet(EthereumWallet::from(signer))
            .connect_http(url)
            .erased();
        Ok(Arc::new(provider))
    }

    /// Build a read-only provider (no signer).
    ///
    /// # Errors
    ///
    /// Returns an error if the RPC URL is invalid.
    pub fn read_provider(&self) -> anyhow::Result<Arc<DynProvider>> {
        let url = reqwest::Url::parse(&self.rpc_url)?;
        let provider = ProviderBuilder::new().connect_http(url).erased();
        Ok(Arc::new(provider))
    }

    /// Look up a contract's deployed address.
    ///
    /// # Errors
    ///
    /// Returns an error if the contract is unknown or the stored address is
    /// not valid hex.
    pub fn address_of(&self, contract: &str) -> anyhow::Result<Address> {
        let hex = self
            .addresses
            .get(contract)
            .ok_or_else(|| anyhow::anyhow!("contract not deployed: {contract}"))?;
        hex.parse()
            .map_err(|e| anyhow::anyhow!("parse address {hex}: {e}"))
    }

    /// Derive the address of the named wallet.
    ///
    /// # Errors
    ///
    /// Returns an error if the wallet is unknown or the private key cannot be
    /// parsed into a signer.
    pub fn wallet_address(&self, name: &str) -> anyhow::Result<Address> {
        let entry = self
            .wallets
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("unknown wallet: {name}"))?;
        let signer: PrivateKeySigner = entry
            .private_key
            .trim_start_matches("0x")
            .parse()
            .map_err(|e| anyhow::anyhow!("derive address: {e}"))?;
        Ok(signer.address())
    }

    /// Return the hex private key for the named wallet.
    pub fn wallet_key(&self, name: &str) -> Option<String> {
        self.wallets.get(name).map(|w| w.private_key.clone())
    }
}
