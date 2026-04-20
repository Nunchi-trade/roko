//! HDC precompile at address `0xA0C`.
//!
//! Solidity surface: [`IHDCPrecompile`](https://github.com/Nunchi-trade/contracts-core/pull/101).
//!
//! # ABI
//!
//! Inputs and outputs follow standard Solidity ABI encoding. `HdcVector` is serialised as
//! a dynamic `bytes` of exactly 1,280 bytes (10,240 bits). `InsightId` is `bytes16`.
//! `f32` similarity / weight / score values are scaled by 1e6 and returned as `uint32`.
//!
//! # State
//!
//! The HNSW index lives in Rust memory (wrapped in `parking_lot::RwLock`) and is **not**
//! visible to EVM storage. Mutations via `insert` / `remove` persist across transactions
//! within the same `ForkState` but are lost across forks (branches) unless explicitly
//! snapshotted. Phase 2 will thread this through `ForkState::clone_for_readonly` etc.
//!
//! # Gas
//!
//! Phase 1 uses a flat per-call cost of 5,000 gas. Phase 2 introduces a size-aware model:
//! similarity is O(HDC_BITS / 64) = 160 u64 popcounts; bundle / bind scale with N.
//! Search is dominated by HNSW traversal — ~4× `k × ef_search` similarity ops on average
//! for the m=16, ef=40 default.

use std::sync::Arc;

use alloy_primitives::{address, Address};
use parking_lot::RwLock;
use revm::{
    context::Cfg,
    context_interface::ContextTr,
    handler::{EthPrecompiles, PrecompileProvider},
    interpreter::{CallInput, CallInputs, Gas, InstructionResult, InterpreterResult},
    primitives::{hardfork::SpecId, Bytes},
};
use roko_primitives::HdcVector;

use crate::chain::{HnswBinaryIndex, HnswConfig};

/// Canonical precompile address — reserved `0xA0C` slot in the Nunchi `0xA00–0xA0F` range.
pub const HDC_PRECOMPILE_ADDRESS: Address = address!("0x0000000000000000000000000000000000000A0C");

/// Length of a packed `HdcVector` in bytes (10,240 bits).
pub const HDC_VECTOR_BYTES: usize = 1_280;

/// Flat Phase-1 gas cost per HDC precompile call. Replaced by a per-method schedule in Phase 2.
const PHASE1_GAS_COST: u64 = 5_000;

// ---- Selectors (keccak256(sig)[..4]) ----
// Signatures taken verbatim from contracts-core `IHDCPrecompile.sol`. Kept as literal
// constants to avoid pulling `alloy-sol-types` into mirage-rs for Phase 1. See the
// `selector_parity_with_solidity_abi` test below for an assertion that these match.

const SELECTOR_PROJECT_BYTES: [u8; 4] = [0xcb, 0xd1, 0x3d, 0x9b]; // projectBytes(bytes)
const SELECTOR_PROJECT_TOKENS: [u8; 4] = [0x1f, 0x7f, 0x97, 0xcb]; // projectTokens(string)
const SELECTOR_BIND: [u8; 4] = [0x67, 0x5f, 0x5f, 0x52]; // bind(bytes,bytes)
const SELECTOR_BUNDLE: [u8; 4] = [0x14, 0x27, 0xc2, 0x92]; // bundle(bytes[])
const SELECTOR_SIMILARITY: [u8; 4] = [0xc6, 0x88, 0xe7, 0x87]; // similarity(bytes,bytes)
const SELECTOR_SEARCH: [u8; 4] = [0x4e, 0xc2, 0xc7, 0x30]; // search(bytes,uint256,uint256)
const SELECTOR_INSERT: [u8; 4] = [0x8e, 0x85, 0xa4, 0x54]; // insert(bytes16,bytes,uint32)
const SELECTOR_REMOVE: [u8; 4] = [0x4c, 0xb7, 0xa7, 0xa2]; // remove(bytes16)

/// Persistent HDC state held by the precompile across EVM invocations.
///
/// `Arc` + `RwLock` so the state can be shared between clones of [`HDCPrecompiles`]
/// (which revm takes by move when re-specing). Phase 2 will move this into `ForkState`.
pub struct HDCState {
    /// HNSW binary index over inserted HDC vectors.
    pub hnsw: RwLock<HnswBinaryIndex>,
}

impl HDCState {
    /// Constructs a fresh empty HDC state with default HNSW parameters.
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            hnsw: RwLock::new(HnswBinaryIndex::new(HnswConfig::default())),
        })
    }
}

impl Default for HDCState {
    fn default() -> Self {
        Self {
            hnsw: RwLock::new(HnswBinaryIndex::new(HnswConfig::default())),
        }
    }
}

/// Custom `PrecompileProvider` that delegates to [`EthPrecompiles`] for standard Ethereum
/// addresses and routes `0xA0C` to the HDC handler.
pub struct HDCPrecompiles {
    eth: EthPrecompiles,
    /// Phase 2 wires `state` into the stateful methods (`insert`, `remove`, `search`).
    #[allow(dead_code)]
    state: Arc<HDCState>,
}

impl HDCPrecompiles {
    /// Construct the combined Ethereum-plus-HDC precompile provider for the given spec.
    #[must_use]
    pub fn new(spec: SpecId, state: Arc<HDCState>) -> Self {
        Self {
            eth: EthPrecompiles::new(spec),
            state,
        }
    }

    /// Dispatch an HDC precompile call by selector. Returns the ABI-encoded output
    /// on success, or an `InterpreterResult` with `InstructionResult::Revert` on failure.
    ///
    /// Takes `&self` to give Phase 2 access to the shared `state` for stateful methods
    /// without refactoring the call sites.
    #[allow(clippy::unused_self)]
    fn run_hdc(&self, input: &[u8], gas_limit: u64) -> InterpreterResult {
        let mut gas = Gas::new(gas_limit);

        // All methods share the same Phase-1 flat cost — refine in Phase 2.
        if !gas.record_cost(PHASE1_GAS_COST) {
            return InterpreterResult {
                result: InstructionResult::PrecompileOOG,
                gas,
                output: Bytes::new(),
            };
        }

        if input.len() < 4 {
            return revert(gas, b"hdc: empty calldata");
        }
        let selector: [u8; 4] = input[..4].try_into().expect("len>=4 checked");
        let payload = &input[4..];

        match selector {
            SELECTOR_SIMILARITY => dispatch_similarity(payload, gas),
            SELECTOR_PROJECT_BYTES
            | SELECTOR_PROJECT_TOKENS
            | SELECTOR_BIND
            | SELECTOR_BUNDLE
            | SELECTOR_SEARCH
            | SELECTOR_INSERT
            | SELECTOR_REMOVE => revert(gas, b"hdc: method not implemented (Phase 2)"),
            _ => revert(gas, b"hdc: unknown selector"),
        }
    }
}

impl<CTX: ContextTr> PrecompileProvider<CTX> for HDCPrecompiles {
    type Output = InterpreterResult;

    fn set_spec(&mut self, spec: <CTX::Cfg as Cfg>::Spec) -> bool {
        <EthPrecompiles as PrecompileProvider<CTX>>::set_spec(&mut self.eth, spec)
    }

    fn run(
        &mut self,
        context: &mut CTX,
        inputs: &CallInputs,
    ) -> Result<Option<Self::Output>, String> {
        if inputs.bytecode_address == HDC_PRECOMPILE_ADDRESS {
            let input_bytes = match &inputs.input {
                CallInput::Bytes(b) => b.0.as_ref(),
                CallInput::SharedBuffer(_) => {
                    // Phase 2: route shared-buffer calldata via context.local().
                    return Ok(Some(revert_str(
                        Gas::new(inputs.gas_limit),
                        "hdc: shared-buffer calldata unsupported (Phase 2)",
                    )));
                }
            };
            let out = self.run_hdc(input_bytes, inputs.gas_limit);
            return Ok(Some(out));
        }
        <EthPrecompiles as PrecompileProvider<CTX>>::run(&mut self.eth, context, inputs)
    }

    fn warm_addresses(&self) -> Box<impl Iterator<Item = Address>> {
        let eth_addrs: Vec<Address> = self.eth.warm_addresses().collect();
        let mut all = eth_addrs;
        all.push(HDC_PRECOMPILE_ADDRESS);
        Box::new(all.into_iter())
    }

    fn contains(&self, address: &Address) -> bool {
        *address == HDC_PRECOMPILE_ADDRESS || self.eth.contains(address)
    }
}

// ---- Method handlers ----

/// `similarity(bytes,bytes) → uint32` — Hamming similarity in `[0, 1e6]`.
fn dispatch_similarity(payload: &[u8], gas: Gas) -> InterpreterResult {
    let Some((a_bytes, b_bytes)) = decode_two_bytes(payload) else {
        return revert(gas, b"hdc.similarity: calldata decode failed");
    };
    if a_bytes.len() != HDC_VECTOR_BYTES || b_bytes.len() != HDC_VECTOR_BYTES {
        return revert(gas, b"hdc.similarity: vector length != 1280");
    }

    let a = HdcVector::from_bytes(a_bytes.try_into().expect("len checked"));
    let b = HdcVector::from_bytes(b_bytes.try_into().expect("len checked"));
    let sim = a.similarity(&b);
    // f32 in [0,1] → uint32 in [0, 1_000_000]. `round` is saturating for NaN (treated as 0).
    let sim_scaled = if sim.is_nan() {
        0u32
    } else {
        (sim.clamp(0.0, 1.0) * 1_000_000.0).round() as u32
    };

    InterpreterResult {
        result: InstructionResult::Return,
        gas,
        output: Bytes::from(encode_uint32(sim_scaled)),
    }
}

// ---- ABI helpers (minimal, hand-rolled) ----

/// Decode two dynamic `bytes` arguments from the payload of `similarity(bytes,bytes)` or
/// `bind(bytes,bytes)`. Returns `None` on any length/bounds failure.
///
/// Solidity ABI layout:
/// - `[0..32]`: offset of arg a (usually 0x40)
/// - `[32..64]`: offset of arg b
/// - `[offset_a..offset_a+32]`: length of a
/// - `[offset_a+32..offset_a+32+len_a]`: data of a (right-padded to 32-byte boundary)
/// - ...same for b
fn decode_two_bytes(payload: &[u8]) -> Option<(&[u8], &[u8])> {
    if payload.len() < 64 {
        return None;
    }
    let offset_a = read_uint_as_usize(&payload[0..32])?;
    let offset_b = read_uint_as_usize(&payload[32..64])?;
    let a = read_bytes_at(payload, offset_a)?;
    let b = read_bytes_at(payload, offset_b)?;
    Some((a, b))
}

/// Read a length-prefixed dynamic `bytes` starting at `offset` within `payload`.
fn read_bytes_at(payload: &[u8], offset: usize) -> Option<&[u8]> {
    if offset.checked_add(32)? > payload.len() {
        return None;
    }
    let len = read_uint_as_usize(&payload[offset..offset + 32])?;
    let data_start = offset + 32;
    let data_end = data_start.checked_add(len)?;
    if data_end > payload.len() {
        return None;
    }
    Some(&payload[data_start..data_end])
}

/// Read a 32-byte big-endian word as `usize`, returning `None` on overflow.
fn read_uint_as_usize(word: &[u8]) -> Option<usize> {
    if word.len() != 32 {
        return None;
    }
    // The high 24 bytes must be zero for the value to fit in usize on 64-bit targets.
    if word[..24].iter().any(|b| *b != 0) {
        return None;
    }
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&word[24..32]);
    Some(u64::from_be_bytes(buf) as usize)
}

/// Encode a `uint32` as a 32-byte big-endian word (standard Solidity ABI).
fn encode_uint32(v: u32) -> Vec<u8> {
    let mut out = vec![0u8; 32];
    out[28..32].copy_from_slice(&v.to_be_bytes());
    out
}

// ---- Revert helpers ----

fn revert(gas: Gas, msg: &[u8]) -> InterpreterResult {
    InterpreterResult {
        result: InstructionResult::Revert,
        gas,
        output: Bytes::copy_from_slice(msg),
    }
}

fn revert_str(gas: Gas, msg: &str) -> InterpreterResult {
    revert(gas, msg.as_bytes())
}

// ---- Tests ----

#[cfg(test)]
mod tests {
    use alloy_primitives::keccak256;

    use super::*;

    /// Assert the hard-coded selectors match the canonical Solidity function signatures.
    /// Any drift here means mirage-rs and contracts-core disagree on the ABI and every
    /// cross-repo call will revert.
    #[test]
    fn selector_parity_with_solidity_abi() {
        let cases: &[(&str, [u8; 4])] = &[
            ("projectBytes(bytes)", SELECTOR_PROJECT_BYTES),
            ("projectTokens(string)", SELECTOR_PROJECT_TOKENS),
            ("bind(bytes,bytes)", SELECTOR_BIND),
            ("bundle(bytes[])", SELECTOR_BUNDLE),
            ("similarity(bytes,bytes)", SELECTOR_SIMILARITY),
            ("search(bytes,uint256,uint256)", SELECTOR_SEARCH),
            ("insert(bytes16,bytes,uint32)", SELECTOR_INSERT),
            ("remove(bytes16)", SELECTOR_REMOVE),
        ];
        for (sig, expected) in cases {
            let hash = keccak256(sig.as_bytes());
            let got: [u8; 4] = hash[..4].try_into().unwrap();
            assert_eq!(got, *expected, "selector mismatch for {sig}");
        }
    }

    /// End-to-end `similarity`: construct two `HdcVector`s, ABI-encode the calldata,
    /// dispatch through the precompile, decode the `uint32` result, and verify it matches
    /// the direct Rust computation.
    #[test]
    fn similarity_round_trip() {
        let a = HdcVector::from_seed(b"agent-a-observation");
        let b = HdcVector::from_seed(b"agent-b-observation");
        let expected = (a.similarity(&b).clamp(0.0, 1.0) * 1_000_000.0).round() as u32;

        let calldata = encode_similarity_calldata(&a, &b);
        let state = HDCState::new();
        let provider = HDCPrecompiles::new(SpecId::SHANGHAI, state);
        let out = provider.run_hdc(&calldata, 1_000_000);

        assert_eq!(out.result, InstructionResult::Return);
        assert_eq!(out.output.len(), 32);
        let got = u32::from_be_bytes(out.output[28..32].try_into().unwrap());
        assert_eq!(got, expected);
    }

    #[test]
    fn similarity_identical_vectors_returns_1e6() {
        let v = HdcVector::from_seed(b"identical");
        let calldata = encode_similarity_calldata(&v, &v);
        let state = HDCState::new();
        let provider = HDCPrecompiles::new(SpecId::SHANGHAI, state);
        let out = provider.run_hdc(&calldata, 1_000_000);
        let got = u32::from_be_bytes(out.output[28..32].try_into().unwrap());
        assert_eq!(got, 1_000_000, "identical vectors should round-trip to 1e6");
    }

    #[test]
    fn similarity_rejects_bad_length() {
        let mut calldata = Vec::new();
        calldata.extend_from_slice(&SELECTOR_SIMILARITY);
        // Offset to arg a at 0x40, arg b at 0x80 (but body will be wrong length)
        calldata.extend_from_slice(&encode_uint32_as_word(0x40));
        calldata.extend_from_slice(&encode_uint32_as_word(0x80));
        // Arg a: length 100, 100 bytes of junk
        calldata.extend_from_slice(&encode_uint32_as_word(100));
        calldata.extend_from_slice(&vec![0u8; 100]);
        // pad to 32-byte boundary (100 % 32 = 4, pad 28 bytes)
        calldata.extend_from_slice(&vec![0u8; 28]);
        // Arg b: length 100
        calldata.extend_from_slice(&encode_uint32_as_word(100));
        calldata.extend_from_slice(&vec![0u8; 128]);

        let state = HDCState::new();
        let provider = HDCPrecompiles::new(SpecId::SHANGHAI, state);
        let out = provider.run_hdc(&calldata, 1_000_000);
        assert_eq!(out.result, InstructionResult::Revert);
    }

    #[test]
    fn unimplemented_methods_revert_with_clear_message() {
        let mut calldata = Vec::new();
        calldata.extend_from_slice(&SELECTOR_PROJECT_BYTES);
        calldata.extend_from_slice(&encode_uint32_as_word(0x20));
        calldata.extend_from_slice(&encode_uint32_as_word(0));

        let state = HDCState::new();
        let provider = HDCPrecompiles::new(SpecId::SHANGHAI, state);
        let out = provider.run_hdc(&calldata, 1_000_000);
        assert_eq!(out.result, InstructionResult::Revert);
        assert!(
            out.output.starts_with(b"hdc: method not implemented"),
            "expected not-implemented revert message, got {:?}",
            out.output
        );
    }

    fn encode_similarity_calldata(a: &HdcVector, b: &HdcVector) -> Vec<u8> {
        let a_bytes = a.to_bytes();
        let b_bytes = b.to_bytes();
        let mut out = Vec::new();
        out.extend_from_slice(&SELECTOR_SIMILARITY);
        // Two offsets: arg a at 0x40, arg b at 0x40 + 32 + 1280 = 0x540
        out.extend_from_slice(&encode_uint32_as_word(0x40));
        out.extend_from_slice(&encode_uint32_as_word(0x40 + 32 + 1280));
        // Arg a: len + data (no padding needed since 1280 % 32 == 0)
        out.extend_from_slice(&encode_uint32_as_word(1280));
        out.extend_from_slice(&a_bytes);
        // Arg b
        out.extend_from_slice(&encode_uint32_as_word(1280));
        out.extend_from_slice(&b_bytes);
        out
    }

    fn encode_uint32_as_word(v: u64) -> [u8; 32] {
        let mut out = [0u8; 32];
        out[24..32].copy_from_slice(&v.to_be_bytes());
        out
    }
}
