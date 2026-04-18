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

#![allow(
    clippy::expect_used,
    clippy::map_unwrap_or,
    clippy::needless_pass_by_value,
    clippy::similar_names,
    clippy::too_many_lines
)]

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
    AssembledContext, CallGraph, CallGraphDirection, CallGraphEdge, CodeIndex, CodeSlice,
    ContextOverlay, EmbeddingQuery, FileAst, HdcQuery, ImplementationMatch, IndexStats,
    KeywordQuery, PrivacyConfig, ReferenceKind, ReferenceMatch, SearchResult, SearchScope,
    StructuralQuery, SymbolContext, SymbolInfo, WorkspaceIndex, WorkspaceMap, WorkspaceMapEntry,
};
