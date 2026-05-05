//! Deterministic in-memory backend for tests, examples, and offline demos.
//!
//! [`MockLightClient`] is the Phase-0 backing for [`crate::TempoLightClient`].
//! It pre-seeds a tiny canned chain (a few headers + a few accounts) so a
//! Roko agent can exercise the full subscribe → verify → read → render flow
//! end-to-end with no network. Real network backings (commonware-p2p
//! follower-node, then LC peer) replace this mock in later phases without
//! changing the [`crate::LightClient`] trait surface.
//!
//! The "verification" performed here is structural — `verify_account` checks
//! that the proof's claimed state root matches the seeded header's state
//! root and that the proof's account fields match what was seeded. It does
//! not hash actual trie nodes. Phase 1 swaps the proof bytes for real
//! commonware-storage trie node walks.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::RwLock;

use crate::error::LcError;
use crate::proof::{AccountProof, AttestationSig, MerkleProof, VerifiedHeader};
use crate::traits::{BlockNumber, LightClient};

/// Canned account record seeded into the mock state.
#[derive(Clone, Debug)]
pub struct MockAccount {
    /// Native balance in wei.
    pub balance_wei: u128,
    /// Account nonce.
    pub nonce: u64,
    /// Code hash (zero hash for EOAs).
    pub code_hash: String,
}

impl Default for MockAccount {
    fn default() -> Self {
        Self {
            balance_wei: 0,
            nonce: 0,
            code_hash: "0x0000000000000000000000000000000000000000000000000000000000000000"
                .to_string(),
        }
    }
}

#[derive(Default)]
struct MockState {
    headers: Vec<VerifiedHeader>,
    /// `(address_lowercase, block) → account record`.
    accounts: HashMap<(String, BlockNumber), MockAccount>,
    /// Cursor into `headers` for `await_next_header` to advance.
    next_header_cursor: usize,
    /// How long `await_next_header` should sleep between ticks.
    tick: Duration,
}

/// In-memory [`LightClient`] backed by pre-seeded headers + accounts.
///
/// Clone-cheap; clones share the same backing state so multiple agents can
/// observe a single mock chain in tests.
#[derive(Clone)]
pub struct MockLightClient {
    state: Arc<RwLock<MockState>>,
    name: Arc<str>,
}

impl MockLightClient {
    /// Empty mock. Use [`MockLightClient::tempo_demo`] for a populated one.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            state: Arc::new(RwLock::new(MockState::default())),
            name: Arc::from(name.into()),
        }
    }

    /// Build a Tempo-shaped demo chain: 5 headers, one well-known address
    /// (the seeded Tempo USDC contract analog) with a per-block balance
    /// curve, and a 200ms cadence. Suitable for the
    /// `examples/tempo_tail.rs` demo.
    ///
    /// The demo address is a deterministic placeholder
    /// (`0x000000000000000000000000000000000000A55E`) — not a real Tempo
    /// deployment. Real addresses come in Phase 1 once Tempo's surface is
    /// canonical (see `docs/08-chain/25-tempo-light-client.md`).
    pub fn tempo_demo() -> Self {
        let lc = Self::new("tempo-mock");
        let demo_addr = "0x000000000000000000000000000000000000a55e";
        for i in 1u64..=5 {
            let header = VerifiedHeader {
                height: i,
                block_hash: hex_padded("blk", i),
                parent_hash: if i == 1 {
                    "0x0000000000000000000000000000000000000000000000000000000000000000".to_string()
                } else {
                    hex_padded("blk", i - 1)
                },
                state_root: hex_padded("root", i),
                timestamp_ms: 1_714_500_000_000 + i * 200,
                attestation: AttestationSig {
                    bytes: vec![0xBE_u8; 32],
                    quorum_id: format!("epoch=1,threshold=67/100,height={i}"),
                },
            };
            lc.push_header(header);
            lc.seed_account(
                demo_addr,
                i,
                MockAccount {
                    balance_wei: 1_000_000_000_000_000_000_u128 * u128::from(i),
                    nonce: i,
                    code_hash: "0xc5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470"
                        .to_string(),
                },
            );
        }
        lc.set_tick(Duration::from_millis(200));
        lc
    }

    /// Seed a header that will be served by [`Self::await_next_header`].
    pub fn push_header(&self, h: VerifiedHeader) {
        self.state.write().headers.push(h);
    }

    /// Seed an account record at a block. Mock proofs commit this record.
    pub fn seed_account(&self, address: &str, block: BlockNumber, acct: MockAccount) {
        self.state
            .write()
            .accounts
            .insert((address.to_lowercase(), block), acct);
    }

    /// Set the cadence at which [`Self::await_next_header`] advances.
    pub fn set_tick(&self, tick: Duration) {
        self.state.write().tick = tick;
    }
}

/// `0x` + label + sequence + trailing-zero pad, so seeded hex-shaped strings
/// are deterministic and exactly 66 chars wide. The strings are mock
/// identifiers — they do not need to be valid hex.
fn hex_padded(label: &str, seq: u64) -> String {
    let core = format!("{label}{seq}");
    let core: String = core.chars().take(64).collect();
    let padding = "0".repeat(64 - core.len());
    format!("0x{core}{padding}")
}

#[async_trait]
impl LightClient for MockLightClient {
    fn name(&self) -> &str {
        &self.name
    }

    fn latest_verified(&self) -> Option<VerifiedHeader> {
        let s = self.state.read();
        if s.next_header_cursor == 0 {
            return None;
        }
        s.headers.get(s.next_header_cursor - 1).cloned()
    }

    async fn await_next_header(&self) -> Result<VerifiedHeader, LcError> {
        let (tick, header) = {
            let mut s = self.state.write();
            if s.next_header_cursor >= s.headers.len() {
                return Err(LcError::Backend("mock: no more seeded headers".into()));
            }
            let h = s.headers[s.next_header_cursor].clone();
            s.next_header_cursor += 1;
            (s.tick, h)
        };
        if !tick.is_zero() {
            tokio::time::sleep(tick).await;
        }
        Ok(header)
    }

    #[allow(clippy::significant_drop_tightening)]
    async fn read_account_at(
        &self,
        address: &str,
        block: BlockNumber,
    ) -> Result<AccountProof, LcError> {
        let key = (address.to_lowercase(), block);
        let (state_root, acct) = {
            let s = self.state.read();
            let header = s
                .headers
                .iter()
                .find(|h| h.height == block)
                .ok_or(LcError::UnknownBlock(block))?;
            let acct = s
                .accounts
                .get(&key)
                .cloned()
                .ok_or_else(|| LcError::UnknownAccount {
                    address: address.to_string(),
                    block,
                })?;
            (header.state_root.clone(), acct)
        };
        let mut node = Vec::with_capacity(128);
        node.extend_from_slice(address.to_lowercase().as_bytes());
        node.extend_from_slice(&block.to_be_bytes());
        node.extend_from_slice(&acct.balance_wei.to_be_bytes());
        node.extend_from_slice(&acct.nonce.to_be_bytes());
        node.extend_from_slice(acct.code_hash.as_bytes());
        node.extend_from_slice(state_root.as_bytes());
        Ok(AccountProof {
            address: address.to_lowercase(),
            block,
            balance_wei: acct.balance_wei,
            nonce: acct.nonce,
            code_hash: acct.code_hash,
            merkle_proof: MerkleProof { nodes: vec![node] },
            against_state_root: state_root,
        })
    }

    #[allow(clippy::significant_drop_tightening)]
    fn verify_account(&self, proof: &AccountProof) -> Result<(), LcError> {
        let key = (proof.address.to_lowercase(), proof.block);
        let s = self.state.read();
        let header_height = s
            .headers
            .iter()
            .find(|h| h.state_root == proof.against_state_root)
            .map(|h| h.height)
            .ok_or_else(|| LcError::InvalidProof {
                state_root: proof.against_state_root.clone(),
            })?;
        if header_height != proof.block {
            return Err(LcError::InvalidProof {
                state_root: proof.against_state_root.clone(),
            });
        }
        let acct = s
            .accounts
            .get(&key)
            .ok_or_else(|| LcError::UnknownAccount {
                address: proof.address.clone(),
                block: proof.block,
            })?;
        if acct.balance_wei != proof.balance_wei
            || acct.nonce != proof.nonce
            || acct.code_hash != proof.code_hash
        {
            return Err(LcError::InvalidProof {
                state_root: proof.against_state_root.clone(),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn demo_chain_advances_and_verifies() {
        let lc = MockLightClient::tempo_demo();
        lc.set_tick(Duration::from_millis(0));

        let mut last_height = 0;
        for _ in 0..5 {
            let h = lc.await_next_header().await.expect("seeded header");
            assert!(h.height > last_height, "height monotone");
            last_height = h.height;

            let proof = lc
                .read_account_at("0x000000000000000000000000000000000000A55E", h.height)
                .await
                .expect("seeded account");
            lc.verify_account(&proof).expect("proof verifies");
            assert_eq!(
                proof.balance_wei,
                1_000_000_000_000_000_000_u128 * u128::from(h.height)
            );
        }

        let err = lc.await_next_header().await.unwrap_err();
        assert!(matches!(err, LcError::Backend(_)));
    }

    #[tokio::test]
    async fn unknown_account_errors_clearly() {
        let lc = MockLightClient::tempo_demo();
        let err = lc
            .read_account_at("0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef", 1)
            .await
            .unwrap_err();
        assert!(matches!(err, LcError::UnknownAccount { .. }));
    }

    #[tokio::test]
    async fn tampered_proof_fails_verification() {
        let lc = MockLightClient::tempo_demo();
        lc.set_tick(Duration::from_millis(0));
        let h = lc.await_next_header().await.unwrap();
        let mut proof = lc
            .read_account_at("0x000000000000000000000000000000000000A55E", h.height)
            .await
            .unwrap();
        proof.balance_wei = proof.balance_wei.wrapping_add(1);
        let err = lc.verify_account(&proof).unwrap_err();
        assert!(matches!(err, LcError::InvalidProof { .. }));
    }

    #[tokio::test]
    async fn latest_verified_tracks_cursor() {
        let lc = MockLightClient::tempo_demo();
        lc.set_tick(Duration::from_millis(0));
        assert!(lc.latest_verified().is_none());
        let h1 = lc.await_next_header().await.unwrap();
        let v1 = lc.latest_verified().unwrap();
        assert_eq!(h1.height, v1.height);
    }
}
