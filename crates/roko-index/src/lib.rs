//! Code intelligence for Roko: source parsing, symbol graphs, and HDC
//! fingerprints.
//!
//! This crate provides the building blocks for code understanding:
//!
//! - **[`parser`]** — Parse source files via a language-agnostic
//!   [`LanguageProvider`](roko_core::LanguageProvider) trait.
//! - **[`symbol`]** — Symbol identifiers, references, and lookup utilities.
//! - **[`graph`]** — Directed symbol dependency graph with `PageRank` scoring.
//! - **[`hdc`]** — 10,240-bit hyperdimensional fingerprints for similarity
//!   search.

pub mod graph;
pub mod hdc;
pub mod parser;
pub mod symbol;

// ─── Convenience re-exports ─────────────────────────────────────────────

pub use graph::{build_graph, pagerank, EdgeKind, SymbolEdge, SymbolGraph};
pub use hdc::{fingerprint_file, fingerprint_symbol, similarity, HdcFingerprint};
pub use parser::{parse_source, SourceFile};
pub use symbol::{find_symbol, SymbolId, SymbolRef};
