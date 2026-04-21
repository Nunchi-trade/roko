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
//! The HDC index lives in Rust memory (wrapped in `parking_lot::RwLock`) and is **not**
//! visible to EVM storage. Mutations via `insert` / `remove` persist across transactions
//! within the same process but are not yet threaded through `ForkState` branching (Phase 3).
//!
//! Backed by [`crate::chain::HdcIndex`] — the brute-force Hamming index — rather than
//! [`crate::chain::HnswBinaryIndex`]. Reason: `HdcIndex` supports `remove`, which HNSW
//! does not, and at the demo's ~100-entry scale brute-force runs in microseconds. HNSW
//! becomes interesting only at 10K+ entries.
//!
//! # Gas
//!
//! Phase 2 uses a flat per-call cost of 5,000 gas. Phase 3 introduces a size-aware model.

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

use crate::chain::{projection, HdcIndex, Hit, InsightId};

/// Canonical precompile address — reserved `0xA0C` slot in the Nunchi `0xA00–0xA0F` range.
pub const HDC_PRECOMPILE_ADDRESS: Address = address!("0x0000000000000000000000000000000000000A0C");

/// Length of a packed `HdcVector` in bytes (10,240 bits).
pub const HDC_VECTOR_BYTES: usize = 1_280;

/// Flat gas cost per HDC precompile call. Refined in Phase 3.
const FLAT_GAS_COST: u64 = 5_000;

/// Maximum `k` accepted by `search` — prevents quadratic encoding cost under adversarial calldata.
const MAX_SEARCH_K: usize = 256;

/// Maximum number of vectors accepted by `bundle` — keeps a single precompile call bounded.
const MAX_BUNDLE_N: usize = 256;

// ---- Selectors (keccak256(sig)[..4]) ----
// Signatures taken verbatim from contracts-core `IHDCPrecompile.sol`. Parity enforced by
// the `selector_parity_with_solidity_abi` test — any drift fails CI.

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
/// `Arc` + `RwLock` so the state can be shared across provider clones. Phase 4 may deep-clone
/// for true COW branching; for now clones share the underlying index.
#[derive(Debug, Default)]
pub struct HDCState {
    /// Brute-force HDC index (id → vector + weight).
    pub index: RwLock<HdcIndex>,
}

impl HDCState {
    /// Constructs a fresh empty HDC state.
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
}

/// Custom `PrecompileProvider` that delegates to [`EthPrecompiles`] for standard Ethereum
/// addresses and routes `0xA0C` to the HDC handler.
pub struct HDCPrecompiles {
    eth: EthPrecompiles,
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
    fn run_hdc(&self, input: &[u8], gas_limit: u64) -> InterpreterResult {
        let mut gas = Gas::new(gas_limit);
        if !gas.record_cost(FLAT_GAS_COST) {
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
            SELECTOR_PROJECT_BYTES => dispatch_project_bytes(payload, gas),
            SELECTOR_PROJECT_TOKENS => dispatch_project_tokens(payload, gas),
            SELECTOR_BIND => dispatch_bind(payload, gas),
            SELECTOR_BUNDLE => dispatch_bundle(payload, gas),
            SELECTOR_SIMILARITY => dispatch_similarity(payload, gas),
            SELECTOR_SEARCH => dispatch_search(payload, gas, &self.state),
            SELECTOR_INSERT => dispatch_insert(payload, gas, &self.state),
            SELECTOR_REMOVE => dispatch_remove(payload, gas, &self.state),
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
                    return Ok(Some(revert_str(
                        Gas::new(inputs.gas_limit),
                        "hdc: shared-buffer calldata unsupported (Phase 3)",
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

fn dispatch_project_bytes(payload: &[u8], gas: Gas) -> InterpreterResult {
    let Some(input) = decode_single_bytes(payload) else {
        return revert(gas, b"hdc.projectBytes: calldata decode failed");
    };
    let vec = projection::project_bytes(input);
    InterpreterResult {
        result: InstructionResult::Return,
        gas,
        output: Bytes::from(encode_dyn_bytes(&vec.to_bytes())),
    }
}

fn dispatch_project_tokens(payload: &[u8], gas: Gas) -> InterpreterResult {
    let Some(input) = decode_single_bytes(payload) else {
        return revert(gas, b"hdc.projectTokens: calldata decode failed");
    };
    let Ok(text) = std::str::from_utf8(input) else {
        return revert(gas, b"hdc.projectTokens: input is not valid UTF-8");
    };
    let vec = projection::project_tokens(text);
    InterpreterResult {
        result: InstructionResult::Return,
        gas,
        output: Bytes::from(encode_dyn_bytes(&vec.to_bytes())),
    }
}

fn dispatch_bind(payload: &[u8], gas: Gas) -> InterpreterResult {
    let Some((a_bytes, b_bytes)) = decode_two_bytes(payload) else {
        return revert(gas, b"hdc.bind: calldata decode failed");
    };
    if a_bytes.len() != HDC_VECTOR_BYTES || b_bytes.len() != HDC_VECTOR_BYTES {
        return revert(gas, b"hdc.bind: vector length != 1280");
    }
    let a = HdcVector::from_bytes(a_bytes.try_into().expect("len checked"));
    let b = HdcVector::from_bytes(b_bytes.try_into().expect("len checked"));
    let out = a.bind(&b);
    InterpreterResult {
        result: InstructionResult::Return,
        gas,
        output: Bytes::from(encode_dyn_bytes(&out.to_bytes())),
    }
}

fn dispatch_bundle(payload: &[u8], gas: Gas) -> InterpreterResult {
    let Some(slices) = decode_bytes_array(payload) else {
        return revert(gas, b"hdc.bundle: calldata decode failed");
    };
    if slices.is_empty() {
        return revert(gas, b"hdc.bundle: empty array");
    }
    if slices.len() > MAX_BUNDLE_N {
        return revert(gas, b"hdc.bundle: too many vectors (max 256)");
    }
    let mut vecs = Vec::with_capacity(slices.len());
    for s in slices {
        if s.len() != HDC_VECTOR_BYTES {
            return revert(gas, b"hdc.bundle: vector length != 1280");
        }
        let arr: &[u8; 1280] = s.try_into().expect("len checked");
        vecs.push(HdcVector::from_bytes(arr));
    }
    let refs: Vec<&HdcVector> = vecs.iter().collect();
    let out = HdcVector::bundle(&refs);
    InterpreterResult {
        result: InstructionResult::Return,
        gas,
        output: Bytes::from(encode_dyn_bytes(&out.to_bytes())),
    }
}

fn dispatch_similarity(payload: &[u8], gas: Gas) -> InterpreterResult {
    let Some((a_bytes, b_bytes)) = decode_two_bytes(payload) else {
        return revert(gas, b"hdc.similarity: calldata decode failed");
    };
    if a_bytes.len() != HDC_VECTOR_BYTES || b_bytes.len() != HDC_VECTOR_BYTES {
        return revert(gas, b"hdc.similarity: vector length != 1280");
    }
    let a = HdcVector::from_bytes(a_bytes.try_into().expect("len checked"));
    let b = HdcVector::from_bytes(b_bytes.try_into().expect("len checked"));
    let sim_scaled = scale_f32_to_uint32(a.similarity(&b));
    InterpreterResult {
        result: InstructionResult::Return,
        gas,
        output: Bytes::from(encode_uint32(sim_scaled)),
    }
}

fn dispatch_search(payload: &[u8], gas: Gas, state: &HDCState) -> InterpreterResult {
    // Calldata: offset_of_bytes (32) | k (32) | ef_search (32)
    if payload.len() < 96 {
        return revert(gas, b"hdc.search: calldata too short");
    }
    let Some(query_offset) = read_uint_as_usize(&payload[0..32]) else {
        return revert(gas, b"hdc.search: query offset decode failed");
    };
    let Some(k) = read_uint_as_usize(&payload[32..64]) else {
        return revert(gas, b"hdc.search: k decode failed");
    };
    // ef_search is accepted but unused by the brute-force HdcIndex (Phase 3 will honour it if we
    // switch to HNSW behind a threshold). We still bounds-check it to avoid silent misuse.
    if read_uint_as_usize(&payload[64..96]).is_none() {
        return revert(gas, b"hdc.search: efSearch decode failed");
    }
    if k == 0 || k > MAX_SEARCH_K {
        return revert(gas, b"hdc.search: k must be in [1, 256]");
    }
    let Some(query_bytes) = read_bytes_at(payload, query_offset) else {
        return revert(gas, b"hdc.search: query decode failed");
    };
    if query_bytes.len() != HDC_VECTOR_BYTES {
        return revert(gas, b"hdc.search: query length != 1280");
    }
    let query = HdcVector::from_bytes(query_bytes.try_into().expect("len checked"));
    let hits = state.index.read().top_k(&query, k);
    InterpreterResult {
        result: InstructionResult::Return,
        gas,
        output: Bytes::from(encode_hits(&hits)),
    }
}

fn dispatch_insert(payload: &[u8], gas: Gas, state: &HDCState) -> InterpreterResult {
    // Calldata head: bytes16 (32) | offset_of_bytes (32) | uint32 (32)
    if payload.len() < 96 {
        return revert(gas, b"hdc.insert: calldata too short");
    }
    let Some(id) = decode_bytes16(&payload[0..32]) else {
        return revert(gas, b"hdc.insert: id decode failed");
    };
    let Some(vec_offset) = read_uint_as_usize(&payload[32..64]) else {
        return revert(gas, b"hdc.insert: vector offset decode failed");
    };
    let Some(weight_scaled) = read_uint32(&payload[64..96]) else {
        return revert(gas, b"hdc.insert: weight decode failed");
    };
    let Some(vec_bytes) = read_bytes_at(payload, vec_offset) else {
        return revert(gas, b"hdc.insert: vector decode failed");
    };
    if vec_bytes.len() != HDC_VECTOR_BYTES {
        return revert(gas, b"hdc.insert: vector length != 1280");
    }
    let vector = HdcVector::from_bytes(vec_bytes.try_into().expect("len checked"));
    let weight_f32 = (weight_scaled as f32) / 1_000_000.0;
    state
        .index
        .write()
        .insert(InsightId(id), vector, weight_f32);
    InterpreterResult {
        result: InstructionResult::Return,
        gas,
        output: Bytes::new(),
    }
}

fn dispatch_remove(payload: &[u8], gas: Gas, state: &HDCState) -> InterpreterResult {
    if payload.len() < 32 {
        return revert(gas, b"hdc.remove: calldata too short");
    }
    let Some(id) = decode_bytes16(&payload[0..32]) else {
        return revert(gas, b"hdc.remove: id decode failed");
    };
    let removed = state.index.write().remove(InsightId(id));
    InterpreterResult {
        result: InstructionResult::Return,
        gas,
        output: Bytes::from(encode_bool(removed)),
    }
}

// ---- ABI helpers (minimal, hand-rolled) ----

fn decode_single_bytes(payload: &[u8]) -> Option<&[u8]> {
    if payload.len() < 32 {
        return None;
    }
    let offset = read_uint_as_usize(&payload[0..32])?;
    read_bytes_at(payload, offset)
}

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

/// Decode a `bytes[]` from payload. Returns `None` on any bounds/length failure.
///
/// Layout:
/// - `[0..32]`: offset to the array body (usually 0x20)
/// - `[offset..offset+32]`: N — the array length
/// - `[offset+32..offset+32+32N]`: N offsets (each relative to `offset + 32`, i.e. the start
///   of the array body after the length word) pointing to each element
/// - Each element: `[elem_start..elem_start+32]` = length, then padded data
fn decode_bytes_array(payload: &[u8]) -> Option<Vec<&[u8]>> {
    if payload.len() < 32 {
        return None;
    }
    let arr_offset = read_uint_as_usize(&payload[0..32])?;
    if arr_offset.checked_add(32)? > payload.len() {
        return None;
    }
    let n = read_uint_as_usize(&payload[arr_offset..arr_offset + 32])?;
    if n > MAX_BUNDLE_N {
        return None;
    }
    let offsets_start = arr_offset + 32;
    if offsets_start.checked_add(n.checked_mul(32)?)? > payload.len() {
        return None;
    }
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let slot = offsets_start + i * 32;
        let rel = read_uint_as_usize(&payload[slot..slot + 32])?;
        let abs = offsets_start.checked_add(rel)?;
        out.push(read_bytes_at(payload, abs)?);
    }
    Some(out)
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

fn read_uint_as_usize(word: &[u8]) -> Option<usize> {
    if word.len() != 32 {
        return None;
    }
    if word[..24].iter().any(|b| *b != 0) {
        return None;
    }
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&word[24..32]);
    Some(u64::from_be_bytes(buf) as usize)
}

fn read_uint32(word: &[u8]) -> Option<u32> {
    if word.len() != 32 {
        return None;
    }
    if word[..28].iter().any(|b| *b != 0) {
        return None;
    }
    Some(u32::from_be_bytes(
        word[28..32].try_into().expect("len checked"),
    ))
}

/// Decode a `bytes16` from a 32-byte word. Solidity `bytesN` is left-aligned with zero padding
/// on the right, so the value is `word[0..16]` and `word[16..32]` must be zero.
fn decode_bytes16(word: &[u8]) -> Option<[u8; 16]> {
    if word.len() != 32 {
        return None;
    }
    if word[16..32].iter().any(|b| *b != 0) {
        return None;
    }
    Some(word[0..16].try_into().expect("len checked"))
}

/// Encode a `uint32` as a 32-byte big-endian word.
fn encode_uint32(v: u32) -> Vec<u8> {
    let mut out = vec![0u8; 32];
    out[28..32].copy_from_slice(&v.to_be_bytes());
    out
}

/// Encode a `bool` as a 32-byte word (`1` or `0`).
fn encode_bool(v: bool) -> Vec<u8> {
    let mut out = vec![0u8; 32];
    out[31] = u8::from(v);
    out
}

/// Encode a dynamic `bytes` return value. Layout: offset pointer (0x20) + length + padded data.
fn encode_dyn_bytes(data: &[u8]) -> Vec<u8> {
    let padded_len = data.len().div_ceil(32) * 32;
    let mut out = Vec::with_capacity(32 + 32 + padded_len);
    // Offset pointer
    out.extend_from_slice(&encode_word_from_usize(0x20));
    // Length
    out.extend_from_slice(&encode_word_from_usize(data.len()));
    // Data + zero padding
    out.extend_from_slice(data);
    out.resize(out.len() + (padded_len - data.len()), 0);
    out
}

/// Encode `Vec<Hit>` as a Solidity `Hit[]` return.
///
/// `Hit` fields (bytes16, uint32, uint32, uint32) are all static, so the tuple has a fixed
/// 128-byte size in the ABI. Array layout: offset pointer (0x20) + length + N × 128 bytes.
fn encode_hits(hits: &[Hit]) -> Vec<u8> {
    let mut out = Vec::with_capacity(32 + 32 + hits.len() * 128);
    out.extend_from_slice(&encode_word_from_usize(0x20));
    out.extend_from_slice(&encode_word_from_usize(hits.len()));
    for hit in hits {
        // bytes16 id — left-aligned, right-padded
        let mut id_word = [0u8; 32];
        id_word[0..16].copy_from_slice(&hit.id.0);
        out.extend_from_slice(&id_word);
        out.extend_from_slice(&encode_uint32(scale_f32_to_uint32(hit.similarity)));
        out.extend_from_slice(&encode_uint32(scale_f32_to_uint32(hit.weight)));
        out.extend_from_slice(&encode_uint32(scale_f32_to_uint32(hit.score)));
    }
    out
}

fn encode_word_from_usize(v: usize) -> [u8; 32] {
    let mut out = [0u8; 32];
    out[24..32].copy_from_slice(&(v as u64).to_be_bytes());
    out
}

/// Convert an `f32` in `[0, 1]` to a `uint32` in `[0, 1_000_000]`. NaN → 0. Weights above 1.0
/// saturate at 1e6 (the Solidity `similarity1e6` contract expects a bounded value).
fn scale_f32_to_uint32(v: f32) -> u32 {
    if v.is_nan() {
        0
    } else {
        (v.clamp(0.0, 1.0) * 1_000_000.0).round() as u32
    }
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

    #[test]
    fn similarity_round_trip() {
        let a = HdcVector::from_seed(b"agent-a-observation");
        let b = HdcVector::from_seed(b"agent-b-observation");
        let expected = scale_f32_to_uint32(a.similarity(&b));
        let out = run(SELECTOR_SIMILARITY, &encode_two_vectors_args(&a, &b));
        assert_eq!(out.result, InstructionResult::Return);
        assert_eq!(u32_from_word(&out.output), expected);
    }

    #[test]
    fn similarity_identical_vectors_returns_1e6() {
        let v = HdcVector::from_seed(b"identical");
        let out = run(SELECTOR_SIMILARITY, &encode_two_vectors_args(&v, &v));
        assert_eq!(u32_from_word(&out.output), 1_000_000);
    }

    #[test]
    fn project_bytes_round_trip() {
        let input = b"agent-42-observation-lorem-ipsum";
        let expected = projection::project_bytes(input);
        let out = run(SELECTOR_PROJECT_BYTES, &encode_single_bytes_arg(input));
        assert_eq!(out.result, InstructionResult::Return);
        let decoded = decode_returned_bytes(&out.output);
        assert_eq!(decoded.len(), 1280);
        assert_eq!(decoded, expected.to_bytes().as_slice());
    }

    #[test]
    fn project_tokens_round_trip() {
        let text = "agent-42 observed a resonance cascade";
        let expected = projection::project_tokens(text);
        let out = run(
            SELECTOR_PROJECT_TOKENS,
            &encode_single_bytes_arg(text.as_bytes()),
        );
        assert_eq!(out.result, InstructionResult::Return);
        let decoded = decode_returned_bytes(&out.output);
        assert_eq!(decoded, expected.to_bytes().as_slice());
    }

    #[test]
    fn project_tokens_rejects_non_utf8() {
        let bad: &[u8] = &[0xff, 0xfe, 0xfd];
        let out = run(SELECTOR_PROJECT_TOKENS, &encode_single_bytes_arg(bad));
        assert_eq!(out.result, InstructionResult::Revert);
        assert!(out.output.starts_with(b"hdc.projectTokens"));
    }

    #[test]
    fn bind_round_trip() {
        let a = HdcVector::from_seed(b"a");
        let b = HdcVector::from_seed(b"b");
        let expected = a.bind(&b);
        let out = run(SELECTOR_BIND, &encode_two_vectors_args(&a, &b));
        assert_eq!(out.result, InstructionResult::Return);
        let decoded = decode_returned_bytes(&out.output);
        assert_eq!(decoded, expected.to_bytes().as_slice());
    }

    #[test]
    fn bundle_three_vectors() {
        let vs = [
            HdcVector::from_seed(b"one"),
            HdcVector::from_seed(b"two"),
            HdcVector::from_seed(b"three"),
        ];
        let refs: Vec<&HdcVector> = vs.iter().collect();
        let expected = HdcVector::bundle(&refs);
        let out = run(SELECTOR_BUNDLE, &encode_bytes_array_arg(&vs));
        assert_eq!(out.result, InstructionResult::Return);
        let decoded = decode_returned_bytes(&out.output);
        assert_eq!(decoded, expected.to_bytes().as_slice());
    }

    #[test]
    fn bundle_rejects_empty_array() {
        let empty: [HdcVector; 0] = [];
        let out = run(SELECTOR_BUNDLE, &encode_bytes_array_arg(&empty));
        assert_eq!(out.result, InstructionResult::Revert);
        assert!(out.output.starts_with(b"hdc.bundle: empty"));
    }

    #[test]
    fn insert_then_remove_round_trip() {
        let state = HDCState::new();
        let provider = HDCPrecompiles::new(SpecId::SHANGHAI, Arc::clone(&state));
        let id = [0x11u8; 16];
        let v = HdcVector::from_seed(b"insert-test");

        // Insert
        let insert_data = encode_insert_args(id, &v, 750_000);
        let out = provider.run_hdc(&with_selector(SELECTOR_INSERT, &insert_data), 1_000_000);
        assert_eq!(out.result, InstructionResult::Return);
        assert_eq!(state.index.read().len(), 1);

        // Remove
        let remove_data = encode_bytes16_arg(id);
        let out = provider.run_hdc(&with_selector(SELECTOR_REMOVE, &remove_data), 1_000_000);
        assert_eq!(out.result, InstructionResult::Return);
        let removed = out.output[31] == 1;
        assert!(removed);
        assert_eq!(state.index.read().len(), 0);
    }

    #[test]
    fn remove_missing_id_returns_false() {
        let state = HDCState::new();
        let provider = HDCPrecompiles::new(SpecId::SHANGHAI, state);
        let out = provider.run_hdc(
            &with_selector(SELECTOR_REMOVE, &encode_bytes16_arg([0xAAu8; 16])),
            1_000_000,
        );
        assert_eq!(out.result, InstructionResult::Return);
        assert_eq!(out.output[31], 0);
    }

    #[test]
    fn search_returns_nearest_neighbour() {
        let state = HDCState::new();
        let provider = HDCPrecompiles::new(SpecId::SHANGHAI, Arc::clone(&state));

        let target = HdcVector::from_seed(b"target");
        let noise_a = HdcVector::from_seed(b"noise-a");
        let noise_b = HdcVector::from_seed(b"noise-b");

        for (id, v) in &[
            ([0x01u8; 16], &target),
            ([0x02u8; 16], &noise_a),
            ([0x03u8; 16], &noise_b),
        ] {
            let data = encode_insert_args(*id, v, 1_000_000);
            let out = provider.run_hdc(&with_selector(SELECTOR_INSERT, &data), 1_000_000);
            assert_eq!(out.result, InstructionResult::Return);
        }

        let args = encode_search_args(&target, 3, 40);
        let out = provider.run_hdc(&with_selector(SELECTOR_SEARCH, &args), 1_000_000);
        assert_eq!(out.result, InstructionResult::Return);
        let hits = decode_hits_from_output(&out.output);
        assert_eq!(hits.len(), 3);
        // Top hit should be the target itself.
        assert_eq!(hits[0].0, [0x01u8; 16]);
        assert_eq!(hits[0].1, 1_000_000); // similarity to self
    }

    #[test]
    fn search_rejects_k_zero() {
        let state = HDCState::new();
        let provider = HDCPrecompiles::new(SpecId::SHANGHAI, state);
        let args = encode_search_args(&HdcVector::from_seed(b"q"), 0, 40);
        let out = provider.run_hdc(&with_selector(SELECTOR_SEARCH, &args), 1_000_000);
        assert_eq!(out.result, InstructionResult::Revert);
        assert!(out.output.starts_with(b"hdc.search: k"));
    }

    #[test]
    fn unknown_selector_reverts() {
        let state = HDCState::new();
        let provider = HDCPrecompiles::new(SpecId::SHANGHAI, state);
        let mut calldata = vec![0xde, 0xad, 0xbe, 0xef];
        calldata.extend_from_slice(&[0u8; 32]);
        let out = provider.run_hdc(&calldata, 1_000_000);
        assert_eq!(out.result, InstructionResult::Revert);
        assert!(out.output.starts_with(b"hdc: unknown selector"));
    }

    // ---- Test helpers ----

    fn run(selector: [u8; 4], payload: &[u8]) -> InterpreterResult {
        let state = HDCState::new();
        let provider = HDCPrecompiles::new(SpecId::SHANGHAI, state);
        provider.run_hdc(&with_selector(selector, payload), 1_000_000)
    }

    fn with_selector(sel: [u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(4 + payload.len());
        out.extend_from_slice(&sel);
        out.extend_from_slice(payload);
        out
    }

    fn encode_single_bytes_arg(data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&encode_word_from_usize(0x20));
        out.extend_from_slice(&encode_word_from_usize(data.len()));
        out.extend_from_slice(data);
        let pad = (32 - (data.len() % 32)) % 32;
        out.resize(out.len() + pad, 0);
        out
    }

    fn encode_two_vectors_args(a: &HdcVector, b: &HdcVector) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&encode_word_from_usize(0x40));
        out.extend_from_slice(&encode_word_from_usize(0x40 + 32 + 1280));
        out.extend_from_slice(&encode_word_from_usize(1280));
        out.extend_from_slice(&a.to_bytes());
        out.extend_from_slice(&encode_word_from_usize(1280));
        out.extend_from_slice(&b.to_bytes());
        out
    }

    fn encode_bytes_array_arg(vecs: &[HdcVector]) -> Vec<u8> {
        // Outer offset
        let mut out = Vec::new();
        out.extend_from_slice(&encode_word_from_usize(0x20));
        // Array length
        out.extend_from_slice(&encode_word_from_usize(vecs.len()));
        // N offsets (relative to the start of the array body = position 0x20 + 32 = 64 in
        // final payload, but the offsets are relative to position just-after-length, so
        // element i starts at (N * 32) + i * (32 + 1280)).
        let offsets_region = vecs.len() * 32;
        for i in 0..vecs.len() {
            let off = offsets_region + i * (32 + 1280);
            out.extend_from_slice(&encode_word_from_usize(off));
        }
        // Elements
        for v in vecs {
            out.extend_from_slice(&encode_word_from_usize(1280));
            out.extend_from_slice(&v.to_bytes());
        }
        out
    }

    fn encode_bytes16_arg(id: [u8; 16]) -> Vec<u8> {
        let mut out = vec![0u8; 32];
        out[0..16].copy_from_slice(&id);
        out
    }

    fn encode_insert_args(id: [u8; 16], v: &HdcVector, weight_1e6: u32) -> Vec<u8> {
        // Head: bytes16 | offset_of_bytes (0x60) | uint32
        let mut out = Vec::new();
        let mut id_word = [0u8; 32];
        id_word[0..16].copy_from_slice(&id);
        out.extend_from_slice(&id_word);
        out.extend_from_slice(&encode_word_from_usize(0x60));
        out.extend_from_slice(&encode_uint32(weight_1e6));
        // Tail: length + 1280 bytes
        out.extend_from_slice(&encode_word_from_usize(1280));
        out.extend_from_slice(&v.to_bytes());
        out
    }

    fn encode_search_args(q: &HdcVector, k: usize, ef_search: usize) -> Vec<u8> {
        // Head: offset_of_bytes (0x60) | k | ef_search
        let mut out = Vec::new();
        out.extend_from_slice(&encode_word_from_usize(0x60));
        out.extend_from_slice(&encode_word_from_usize(k));
        out.extend_from_slice(&encode_word_from_usize(ef_search));
        // Tail
        out.extend_from_slice(&encode_word_from_usize(1280));
        out.extend_from_slice(&q.to_bytes());
        out
    }

    fn u32_from_word(word: &[u8]) -> u32 {
        u32::from_be_bytes(word[28..32].try_into().unwrap())
    }

    /// Decode a dynamic `bytes` returned from the precompile.
    fn decode_returned_bytes(output: &[u8]) -> &[u8] {
        let offset = read_uint_as_usize(&output[0..32]).unwrap();
        let len = read_uint_as_usize(&output[offset..offset + 32]).unwrap();
        &output[offset + 32..offset + 32 + len]
    }

    /// Decode `Hit[]` as `Vec<([u8; 16], sim1e6, weight1e6, score1e6)>`.
    fn decode_hits_from_output(output: &[u8]) -> Vec<([u8; 16], u32, u32, u32)> {
        let offset = read_uint_as_usize(&output[0..32]).unwrap();
        let n = read_uint_as_usize(&output[offset..offset + 32]).unwrap();
        let mut hits = Vec::with_capacity(n);
        let mut cursor = offset + 32;
        for _ in 0..n {
            let id: [u8; 16] = output[cursor..cursor + 16].try_into().unwrap();
            let sim = u32_from_word(&output[cursor + 32..cursor + 64]);
            let w = u32_from_word(&output[cursor + 64..cursor + 96]);
            let s = u32_from_word(&output[cursor + 96..cursor + 128]);
            hits.push((id, sim, w, s));
            cursor += 128;
        }
        hits
    }
}
