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
pub mod workspace;

// ─── Convenience re-exports ─────────────────────────────────────────────

pub use graph::{EdgeKind, SymbolEdge, SymbolGraph, build_graph, pagerank};
pub use hdc::{HdcFingerprint, fingerprint_file, fingerprint_symbol, similarity};
pub use parser::{SourceFile, parse_source};
pub use symbol::{SymbolId, SymbolRef, find_symbol};
pub use workspace::{
    CallGraph, CallGraphDirection, CallGraphEdge, SearchResult, SymbolInfo, WorkspaceIndex,
};
