//! EIP-1186 account-proof verification.
//!
//! Walks the Merkle Patricia trie nodes returned by `eth_getProof` from the
//! leaf to the trie root, hashes each, and confirms the recovered root
//! matches the `stateRoot` claimed by the verified header.
//!
//! Trust model: this verifies that the account record committed in the
//! `state_root` is exactly the one in the proof — i.e. the RPC operator
//! cannot lie about an account's balance / nonce / code without producing
//! an invalid proof. The trust gap that remains is whether the *header* and
//! its `state_root` are themselves real (Tempo's BLS aggregate signature
//! over the consensus output). That gap is what Phase 2 closes.

use alloy_primitives::{B256, Bytes, U256, keccak256};
use alloy_rlp::Encodable;
use alloy_trie::{Nibbles, TrieAccount, proof::verify_proof};

use crate::error::LcError;
use crate::proof::AccountProof;

/// `keccak256("")` — the canonical empty-code hash. Returned by `eth_getProof`
/// for accounts that have never been touched (EOAs that never received funds).
const EMPTY_CODE_HASH: [u8; 32] = [
    0xc5, 0xd2, 0x46, 0x01, 0x86, 0xf7, 0x23, 0x3c, 0x92, 0x7e, 0x7d, 0xb2, 0xdc, 0xc7, 0x03, 0xc0,
    0xe5, 0x00, 0xb6, 0x53, 0xca, 0x82, 0x27, 0x3b, 0x7b, 0xfa, 0xd8, 0x04, 0x5d, 0x85, 0xa4, 0x70,
];

/// `keccak256(rlp(""))` — the canonical empty-storage-trie root.
const EMPTY_TRIE_ROOT: [u8; 32] = [
    0x56, 0xe8, 0x1f, 0x17, 0x1b, 0xcc, 0x55, 0xa6, 0xff, 0x83, 0x45, 0xe6, 0x92, 0xc0, 0xf8, 0x6e,
    0x5b, 0x48, 0xe0, 0x1b, 0x99, 0x6c, 0xad, 0xc0, 0x01, 0x62, 0x2f, 0xb5, 0xe3, 0x63, 0xb4, 0x21,
];

/// Verify an [`AccountProof`] against its claimed `against_state_root`.
///
/// The proof is the array of RLP-encoded MPT nodes that `eth_getProof` returned.
/// The trie key is `keccak256(address)`; the leaf value (if present) is
/// `rlp([nonce, balance, storage_root, code_hash])`.
///
/// # Errors
///
/// Returns [`LcError::InvalidProof`] if the proof does not walk to the claimed
/// state root, or if the on-chain account record is not the one named in the
/// proof. Returns [`LcError::Backend`] if any of the hex strings in the proof
/// are malformed.
pub fn verify_account_proof(proof: &AccountProof) -> Result<(), LcError> {
    let address_bytes =
        parse_hex_to_vec(&proof.address).map_err(|e| LcError::Backend(format!("address: {e}")))?;
    let state_root = parse_b256(&proof.against_state_root)
        .map_err(|e| LcError::Backend(format!("state_root: {e}")))?;
    let storage_root = parse_b256(&proof.storage_hash)
        .map_err(|e| LcError::Backend(format!("storage_hash: {e}")))?;
    let code_hash =
        parse_b256(&proof.code_hash).map_err(|e| LcError::Backend(format!("code_hash: {e}")))?;

    let key = Nibbles::unpack(keccak256(&address_bytes));

    let expected_value = if is_empty_account(
        proof.nonce,
        proof.balance_wei,
        storage_root.as_slice(),
        code_hash.as_slice(),
    ) {
        None
    } else {
        let account = TrieAccount {
            nonce: proof.nonce,
            balance: U256::from(proof.balance_wei),
            storage_root,
            code_hash,
        };
        let mut buf = Vec::with_capacity(96);
        account.encode(&mut buf);
        Some(buf)
    };

    let nodes: Vec<Bytes> = proof
        .merkle_proof
        .nodes
        .iter()
        .map(|v| Bytes::from(v.clone()))
        .collect();

    verify_proof(state_root, key, expected_value, nodes.iter()).map_err(|_| LcError::InvalidProof {
        state_root: proof.against_state_root.clone(),
    })
}

fn is_empty_account(nonce: u64, balance: u128, storage_root: &[u8], code_hash: &[u8]) -> bool {
    nonce == 0 && balance == 0 && storage_root == EMPTY_TRIE_ROOT && code_hash == EMPTY_CODE_HASH
}

fn parse_b256(s: &str) -> Result<B256, String> {
    s.parse::<B256>().map_err(|e| format!("{s}: {e}"))
}

fn parse_hex_to_vec(s: &str) -> Result<Vec<u8>, String> {
    let trimmed = s.strip_prefix("0x").unwrap_or(s);
    hex::decode(trimmed).map_err(|e| format!("{s}: {e}"))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn empty_account_detection() {
        assert!(is_empty_account(0, 0, &EMPTY_TRIE_ROOT, &EMPTY_CODE_HASH));
        assert!(!is_empty_account(1, 0, &EMPTY_TRIE_ROOT, &EMPTY_CODE_HASH));
        assert!(!is_empty_account(0, 1, &EMPTY_TRIE_ROOT, &EMPTY_CODE_HASH));
    }

    #[test]
    fn parse_b256_roundtrip() {
        let s = "0x0000000000000000000000000000000000000000000000000000000000000001";
        let h = parse_b256(s).unwrap();
        assert_eq!(format!("{h:#x}"), s);
    }
}
