//! Custom revm precompiles injected by mirage-rs.
//!
//! Gated behind the `chain` cargo feature — the HDC precompile depends on
//! [`crate::chain`] types (`HnswBinaryIndex`, `HdcVector`, projection helpers).
//!
//! # Current state (Phase 1 scaffolding)
//!
//! - [`hdc::HDCPrecompiles`] wraps the default [`revm::EthPrecompiles`] and adds a single
//!   custom address at `0xA0C` (the HDC precompile).
//! - **`similarity`** is fully implemented end-to-end (pure, stateless — decodes two
//!   ABI-encoded `HdcVector`s, computes Hamming similarity via
//!   [`roko_primitives::HdcVector::similarity`], encodes the `f32` as `uint32` scaled by 1e6).
//! - The remaining 7 methods (`projectBytes`, `projectTokens`, `bind`, `bundle`, `search`,
//!   `insert`, `remove`) are stubbed — they return a `NotImplemented` revert so the call
//!   surface compiles against [`packages/agents/src/IHDCPrecompile.sol`] in `contracts-core`
//!   but doesn't yet exercise the underlying Rust math.
//! - **Not yet wired** into `fork.rs` — the three `build_mainnet()` call sites (lines
//!   1582, 1933, 1989) still use the default `EthPrecompiles`. Phase 2 replaces them
//!   with a `build_mirage()` helper that constructs `Evm` with `HDCPrecompiles` instead.
//!
//! Tracking issue: see the PR description on `Nunchi-trade/roko`.

#[cfg(feature = "chain")]
pub mod hdc;
