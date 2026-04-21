//! Workspace-level index built from parsed source files.
//!
//! The low-level `parser`, `graph`, and `hdc` modules are intentionally
//! composable primitives. This module stitches them into a repository index
//! that can answer higher-level code-intelligence queries.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, anyhow, bail};
use roko_core::language::{Import, LanguageProvider, SymbolKind, Visibility};
use roko_lang_go::GoLanguageProvider;
use roko_lang_rust::RustLanguageProvider;
use roko_lang_typescript::TypeScriptLanguageProvider;
use serde::{Deserialize, Serialize};

use crate::graph::{EdgeKind, SymbolGraph, build_graph, pagerank};
use crate::hdc::{fingerprint_file, fingerprint_symbol, similarity};
use crate::parser::{SourceFile, parse_source};
use crate::symbol::{SymbolId, SymbolRef};

static RUST_PROVIDER: RustLanguageProvider = RustLanguageProvider;
static TS_PROVIDER: TypeScriptLanguageProvider = TypeScriptLanguageProvider;
static GO_PROVIDER: GoLanguageProvider = GoLanguageProvider;

/// Serializable symbol summary for MCP responses.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "rkyv",
    derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)
)]
pub struct SymbolInfo {
    /// Stable symbol identifier.
    pub id: SymbolId,
    /// Visibility of the symbol.
    pub visibility: Visibility,
    /// Line where the symbol is defined.
    pub line: usize,
    /// Language inferred for the file that defines the symbol.
    pub language: String,
}

/// Direction of a call-graph edge.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallGraphDirection {
    /// A caller reaches its callee.
    Callees,
    /// A callee is reached by its callers.
    Callers,
}

/// A single call-graph edge.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallGraphEdge {
    /// Source symbol.
    pub from: SymbolInfo,
    /// Target symbol.
    pub to: SymbolInfo,
    /// Direction from the query root.
    pub direction: CallGraphDirection,
    /// Hop distance from the query root.
    pub depth: usize,
}

/// Call graph neighbourhood around one or more matching symbols.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallGraph {
    /// Function name requested by the caller.
    pub function: String,
    /// Maximum traversal depth.
    pub depth: usize,
    /// Symbols matching the requested function name.
    pub roots: Vec<SymbolInfo>,
    /// Callers reachable within `depth`.
    pub callers: Vec<SymbolInfo>,
    /// Callees reachable within `depth`.
    pub callees: Vec<SymbolInfo>,
    /// Traversed edges.
    pub edges: Vec<CallGraphEdge>,
}

/// Semantic search hit.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SearchResult {
    /// Similarity score in `[0.0, 1.0]`.
    pub score: f64,
    /// Symbol matched by the query.
    pub symbol: SymbolInfo,
}

/// Scope for keyword-oriented code search.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchScope {
    /// Search symbol names only.
    Symbols,
    /// Search file paths only.
    Files,
    /// Search both symbol names and file paths.
    #[default]
    Both,
}

/// Keyword search configuration for the in-memory code index.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeywordQuery {
    /// Text to search for.
    pub text: String,
    /// Which indexed fields to inspect.
    #[serde(default)]
    pub scope: SearchScope,
    /// Whether comparisons should preserve case.
    #[serde(default)]
    pub case_sensitive: bool,
    /// Whether only exact whole-field matches should be returned.
    #[serde(default)]
    pub whole_word: bool,
}

/// Structural search filters for indexed symbols.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct StructuralQuery {
    /// Restrict matches to one symbol kind.
    pub kind: Option<SymbolKind>,
    /// Restrict matches to one visibility level.
    pub visibility: Option<Visibility>,
    /// Restrict matches to files whose path matches this glob-like pattern.
    pub file_pattern: Option<String>,
    /// Require symbols to have callers or no callers.
    pub has_callers: Option<bool>,
    /// Require a minimum `PageRank` score.
    pub min_pagerank: Option<f64>,
}

/// HDC similarity search query.
#[derive(Clone, Debug, PartialEq)]
pub struct HdcQuery {
    /// Fingerprint used as the search anchor.
    pub anchor: crate::hdc::HdcFingerprint,
    /// Minimum similarity score in `[0.0, 1.0]`.
    pub min_similarity: f64,
    /// Maximum number of results to return.
    pub max_results: usize,
}

/// Dense-embedding search query.
///
/// The current in-memory implementation does not compute dense embeddings yet;
/// callers can still use this type to describe the intended search.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EmbeddingQuery {
    /// Natural-language or code query text.
    pub text: String,
    /// Precomputed embedding vector.
    pub embedding: Vec<f32>,
    /// Maximum number of results to return.
    pub max_results: usize,
    /// Minimum similarity threshold in `[0.0, 1.0]`.
    pub min_similarity: f32,
}

/// A unified search strategy combining one or more search dimensions.
#[derive(Clone, Debug)]
pub enum SearchStrategy {
    /// Single keyword/text search.
    Keyword(KeywordQuery),
    /// Filter by symbol structure (kind, visibility, file pattern).
    Structural(StructuralQuery),
    /// HDC similarity search.
    Hdc(HdcQuery),
    /// Hybrid search combining multiple strategies with RRF ranking.
    Hybrid {
        /// Optional keyword sub-query.
        keyword: Option<KeywordQuery>,
        /// Optional structural sub-query.
        structural: Option<StructuralQuery>,
        /// Optional HDC sub-query.
        hdc: Option<HdcQuery>,
    },
}

/// Extracted code slice for prompt assembly or MCP responses.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeSlice {
    /// File that contributed the slice.
    pub file_path: String,
    /// First 1-based line included in the slice.
    pub start_line: usize,
    /// Last 1-based line included in the slice.
    pub end_line: usize,
    /// Extracted source text.
    pub content: String,
    /// Symbols covered by the slice.
    pub symbols_included: Vec<SymbolId>,
    /// Approximate token count for the slice.
    pub token_estimate: usize,
}

/// Per-agent index view customization.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ContextOverlay {
    /// Files that should always be preferred.
    pub pinned_files: Vec<String>,
    /// File patterns that should be excluded.
    pub excluded_patterns: Vec<String>,
    /// Importance multipliers keyed by symbol id.
    pub importance_overrides: HashMap<SymbolId, f64>,
    /// Maximum graph expansion depth.
    pub max_expansion_depth: usize,
}

/// Privacy controls applied during context assembly.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrivacyConfig {
    /// String patterns to redact from emitted context.
    pub redact_patterns: Vec<String>,
    /// File paths that should be excluded entirely.
    pub ignore_files: Vec<String>,
    /// Symbol names that should be excluded entirely.
    pub blocked_symbols: Vec<String>,
}

/// Relationship basis used when reporting references.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReferenceKind {
    /// The symbol definition itself.
    Definition,
    /// A graph edge induced by an import.
    Imports,
    /// A graph edge induced by a call.
    Calls,
    /// A graph edge induced by an implementation relation.
    Implements,
    /// A graph edge induced by containment.
    Contains,
    /// A graph edge induced by a type reference.
    TypeRef,
}

/// Reference-like location returned by the code index.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReferenceMatch {
    /// Relationship connecting this location to the queried symbol.
    pub kind: ReferenceKind,
    /// Symbol that references the queried symbol.
    pub symbol: SymbolInfo,
    /// Location currently available for the reference.
    pub location: SymbolRef,
}

/// Structured symbol context used by MCP and prompt assembly.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SymbolContext {
    /// Resolved symbol.
    pub symbol: SymbolInfo,
    /// Approximate `PageRank` score for the symbol.
    pub pagerank: f64,
    /// Parsed imports for the symbol's file.
    pub imports: Vec<Import>,
    /// Direct dependencies of the symbol.
    pub dependencies: Vec<SymbolInfo>,
    /// Direct callers of the symbol.
    pub callers: Vec<SymbolInfo>,
    /// Definition slice for the symbol.
    pub definition: Option<CodeSlice>,
}

/// Symbol-level structure of one source file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileAst {
    /// File path resolved within the workspace.
    pub file_path: String,
    /// Language provider assigned to the file.
    pub language: String,
    /// Parsed imports in the file.
    pub imports: Vec<Import>,
    /// Top-level symbols in source order.
    pub symbols: Vec<SymbolInfo>,
    /// Number of source lines in the file.
    pub line_count: usize,
}

/// Summary statistics for the current index.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct IndexStats {
    /// Number of indexed source files.
    pub indexed_files: usize,
    /// Number of indexed symbols.
    pub total_symbols: usize,
    /// Number of graph edges.
    pub total_edges: usize,
    /// Edge counts grouped by type.
    pub edge_breakdown: HashMap<String, usize>,
    /// Indexed files grouped by language.
    pub languages: HashMap<String, usize>,
    /// Top-ranked symbols by `PageRank`.
    pub top_symbols_by_pagerank: Vec<SearchResult>,
}

/// Trait or interface implementation discovered in the workspace.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImplementationMatch {
    /// Trait or interface name.
    pub trait_name: String,
    /// Type that implements the trait.
    pub implementor: String,
    /// Impl block symbol.
    pub implementation: SymbolInfo,
    /// Methods heuristically associated with the impl block.
    pub methods: Vec<SymbolInfo>,
}

/// File-level entry in a workspace map.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceMapEntry {
    /// Workspace-relative file path.
    pub file_path: String,
    /// Language of the file.
    pub language: String,
    /// Number of symbols found in the file.
    pub symbol_count: usize,
    /// Representative symbol names.
    pub symbols: Vec<String>,
}

/// Structural overview of the indexed workspace.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceMap {
    /// Workspace root used to resolve the map.
    pub root: String,
    /// Files included in the map.
    pub files: Vec<WorkspaceMapEntry>,
}

/// Budgeted code context assembled from search results.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssembledContext {
    /// Original task description.
    pub query: String,
    /// Slices selected for the context window.
    pub slices: Vec<CodeSlice>,
    /// Total token estimate for all slices.
    pub token_estimate: usize,
    /// Whether additional slices were omitted to satisfy the budget.
    pub truncated: bool,
}

/// Common interface implemented by code-index backends.
pub trait CodeIndex {
    /// Look up a symbol by exact name.
    fn lookup_symbol(&self, name: &str) -> Vec<SymbolInfo>;

    /// Search the index using symbol/file keyword matching.
    fn search_by_keyword(&self, query: &KeywordQuery, limit: usize) -> Vec<SearchResult>;

    /// Search the index using structural filters.
    fn search_by_structure(&self, query: &StructuralQuery, limit: usize) -> Vec<SearchResult>;

    /// Search the index using HDC similarity.
    fn search_by_fingerprint(&self, query: &HdcQuery) -> Vec<SearchResult>;

    /// Return semantic matches for a natural-language query.
    fn search_by_embedding(&self, query: &EmbeddingQuery) -> Vec<SearchResult>;

    /// Return the parsed imports for one file.
    fn list_imports_for_file(&self, file: &str) -> Result<Vec<Import>>;

    /// Return a symbol-centric context bundle.
    fn build_symbol_context(
        &self,
        symbol_name: &str,
        file_path: Option<&str>,
        expansion_depth: usize,
    ) -> Result<Vec<SymbolContext>>;

    /// Return the call graph neighborhood around a function.
    fn find_call_graph(&self, function: &str, depth: u32) -> CallGraph;

    /// Return the file-level structure for one file.
    fn file_ast(&self, file: &str) -> Result<FileAst>;

    /// Return the current state of the index.
    fn index_stats(&self) -> IndexStats;

    /// Find graph-backed references to a symbol.
    fn find_references(
        &self,
        symbol_name: &str,
        file_path: Option<&str>,
        include_definitions: bool,
    ) -> Result<Vec<ReferenceMatch>>;

    /// Find implementations of a trait or interface.
    fn find_implementations(&self, trait_name: &str) -> Vec<ImplementationMatch>;

    /// Produce a structural workspace overview.
    fn workspace_map(&self, focus: Option<&str>) -> WorkspaceMap;

    /// Assemble a budgeted code context for a task.
    fn assemble_context(
        &self,
        query: &str,
        max_results: usize,
        token_budget: usize,
        overlay: Option<&ContextOverlay>,
        privacy: Option<&PrivacyConfig>,
    ) -> AssembledContext;
}

/// Workspace-wide code index.
#[derive(Clone, Debug)]
pub struct WorkspaceIndex {
    root: PathBuf,
    files_by_path: HashMap<String, SourceFile>,
    file_paths: HashSet<String>,
    imports_by_file: HashMap<String, Vec<Import>>,
    symbols_by_name: HashMap<String, Vec<SymbolInfo>>,
    functions_by_name: HashMap<String, Vec<SymbolInfo>>,
    symbols_by_id: HashMap<SymbolId, SymbolInfo>,
    file_fingerprints: HashMap<String, crate::hdc::HdcFingerprint>,
    symbol_fingerprints: HashMap<SymbolId, crate::hdc::HdcFingerprint>,
    pagerank_scores: HashMap<SymbolId, f64>,
    graph: SymbolGraph,
}

impl WorkspaceIndex {
    /// Build an index from a workspace root.
    ///
    /// # Errors
    ///
    /// Returns an error if the workspace root cannot be canonicalized or its
    /// source files cannot be collected and parsed.
    pub fn load(root: impl AsRef<Path>) -> Result<Self> {
        let root = std::fs::canonicalize(root.as_ref())
            .with_context(|| format!("resolve workspace root {}", root.as_ref().display()))?;
        let files = collect_source_files(&root)?;
        Ok(Self::from_source_files_with_root(root, files))
    }

    /// Build an index from already-parsed source files.
    pub fn from_source_files(files: Vec<SourceFile>) -> Self {
        Self::from_source_files_with_root(PathBuf::from("."), files)
    }

    /// Resolve a symbol name across the workspace.
    pub fn symbol_lookup(&self, name: &str) -> Vec<SymbolInfo> {
        self.symbols_by_name.get(name).cloned().unwrap_or_default()
    }

    /// Search indexed symbols and files using keyword matching.
    pub fn keyword_search(&self, query: &KeywordQuery, limit: usize) -> Vec<SearchResult> {
        if limit == 0 {
            return Vec::new();
        }

        let needle = normalize_match_field(&query.text, query.case_sensitive);
        let mut results = self
            .symbols_by_id
            .values()
            .filter_map(|symbol| {
                let symbol_name =
                    normalize_match_field(&symbol.id.symbol_name, query.case_sensitive);
                let file_path = normalize_match_field(&symbol.id.file_path, query.case_sensitive);
                let matched = match query.scope {
                    SearchScope::Symbols => {
                        keyword_matches(&symbol_name, &needle, query.whole_word)
                    }
                    SearchScope::Files => keyword_matches(&file_path, &needle, query.whole_word),
                    SearchScope::Both => {
                        keyword_matches(&symbol_name, &needle, query.whole_word)
                            || keyword_matches(&file_path, &needle, query.whole_word)
                    }
                };
                if !matched {
                    return None;
                }

                let mut score = if symbol_name == needle {
                    1.0
                } else if symbol_name.starts_with(&needle) {
                    0.95
                } else if file_path == needle {
                    0.9
                } else {
                    0.8
                };
                score += self.pagerank(&symbol.id).min(0.2);

                Some(SearchResult {
                    score,
                    symbol: symbol.clone(),
                })
            })
            .collect::<Vec<_>>();

        sort_search_results(&mut results);
        results.truncate(limit);
        results
    }

    /// Search indexed symbols using structural filters.
    pub fn structural_search(&self, query: &StructuralQuery, limit: usize) -> Vec<SearchResult> {
        if limit == 0 {
            return Vec::new();
        }

        let mut results = self
            .symbols_by_id
            .values()
            .filter_map(|symbol| {
                if let Some(kind) = &query.kind {
                    if symbol.id.kind != *kind {
                        return None;
                    }
                }
                if let Some(visibility) = &query.visibility {
                    if symbol.visibility != *visibility {
                        return None;
                    }
                }
                if let Some(pattern) = &query.file_pattern {
                    if !matches_file_pattern(&symbol.id.file_path, pattern) {
                        return None;
                    }
                }
                if let Some(has_callers) = query.has_callers {
                    let callers = !self
                        .graph
                        .reverse_neighbors_by_kind(&symbol.id, EdgeKind::Calls)
                        .is_empty();
                    if callers != has_callers {
                        return None;
                    }
                }

                let score = self.pagerank(&symbol.id);
                if let Some(min_pagerank) = query.min_pagerank {
                    if score < min_pagerank {
                        return None;
                    }
                }

                Some(SearchResult {
                    score,
                    symbol: symbol.clone(),
                })
            })
            .collect::<Vec<_>>();

        sort_search_results(&mut results);
        results.truncate(limit);
        results
    }

    /// Search indexed symbols using an HDC fingerprint.
    pub fn hdc_search(&self, query: &HdcQuery) -> Vec<SearchResult> {
        if query.max_results == 0 {
            return Vec::new();
        }

        let mut results = self
            .symbol_fingerprints
            .iter()
            .filter_map(|(id, fingerprint)| {
                let score = similarity(&query.anchor, fingerprint);
                if score < query.min_similarity {
                    return None;
                }
                self.symbols_by_id
                    .get(id)
                    .cloned()
                    .map(|symbol| SearchResult { score, symbol })
            })
            .collect::<Vec<_>>();

        sort_search_results(&mut results);
        results.truncate(query.max_results);
        results
    }

    /// Dense-embedding search fallback.
    ///
    /// The current implementation falls back to keyword matching on the query
    /// text because this backend does not yet compute or store embeddings.
    pub fn embedding_search(&self, query: &EmbeddingQuery) -> Vec<SearchResult> {
        let fallback = KeywordQuery {
            text: query.text.clone(),
            scope: SearchScope::Both,
            case_sensitive: false,
            whole_word: false,
        };
        self.keyword_search(&fallback, query.max_results)
            .into_iter()
            .filter(|result| result.score >= f64::from(query.min_similarity))
            .collect()
    }

    /// Unified search dispatching to the appropriate strategy.
    ///
    /// For `SearchStrategy::Hybrid`, results from each sub-strategy are merged
    /// using Reciprocal Rank Fusion (RRF).
    pub fn search(&self, strategy: SearchStrategy, limit: usize) -> Vec<SearchResult> {
        match strategy {
            SearchStrategy::Keyword(query) => self.keyword_search(&query, limit),
            SearchStrategy::Structural(query) => self.structural_search(&query, limit),
            SearchStrategy::Hdc(query) => {
                let mut q = query;
                q.max_results = limit;
                self.hdc_search(&q)
            }
            SearchStrategy::Hybrid {
                keyword,
                structural,
                hdc,
            } => {
                let oversample = limit.saturating_mul(3).max(30);
                let mut lists: Vec<Vec<SearchResult>> = Vec::new();
                if let Some(q) = keyword {
                    lists.push(self.keyword_search(&q, oversample));
                }
                if let Some(q) = structural {
                    lists.push(self.structural_search(&q, oversample));
                }
                if let Some(q) = hdc {
                    let mut q = q;
                    q.max_results = oversample;
                    lists.push(self.hdc_search(&q));
                }
                rrf_merge(&lists, 60.0, limit)
            }
        }
    }

    /// Return the imports for a file.
    ///
    /// # Errors
    ///
    /// Returns an error if `file` cannot be resolved to a file tracked by this
    /// workspace index.
    pub fn imports_for_file(&self, file: &str) -> Result<Vec<Import>> {
        let key = self.resolve_file_key(file)?;
        Ok(self.imports_by_file.get(&key).cloned().unwrap_or_default())
    }

    /// Return direct symbol context for an exact symbol lookup.
    ///
    /// # Errors
    ///
    /// Returns an error if `file_path` is provided and does not resolve within
    /// the index.
    pub fn symbol_context(
        &self,
        symbol_name: &str,
        file_path: Option<&str>,
        expansion_depth: usize,
    ) -> Result<Vec<SymbolContext>> {
        let matches = self.resolve_symbols(symbol_name, file_path)?;
        Ok(matches
            .into_iter()
            .map(|symbol| SymbolContext {
                imports: self
                    .imports_by_file
                    .get(&symbol.id.file_path)
                    .cloned()
                    .unwrap_or_default(),
                dependencies: self.related_symbols(&symbol.id, expansion_depth, false),
                callers: self.related_symbols(&symbol.id, expansion_depth, true),
                definition: self.slice_for_symbol(&symbol.id),
                pagerank: self.pagerank(&symbol.id),
                symbol,
            })
            .collect())
    }

    /// Return a call graph neighborhood for a function name.
    pub fn call_graph(&self, function: &str, depth: u32) -> CallGraph {
        let roots = self
            .functions_by_name
            .get(function)
            .cloned()
            .unwrap_or_default();
        let depth = depth as usize;
        if roots.is_empty() {
            return CallGraph {
                function: function.to_string(),
                depth,
                roots,
                callers: Vec::new(),
                callees: Vec::new(),
                edges: Vec::new(),
            };
        }

        let (callers, caller_edges) =
            self.traverse_call_graph(&roots, depth, CallGraphDirection::Callers);
        let (callees, callee_edges) =
            self.traverse_call_graph(&roots, depth, CallGraphDirection::Callees);

        CallGraph {
            function: function.to_string(),
            depth,
            roots,
            callers,
            callees,
            edges: caller_edges.into_iter().chain(callee_edges).collect(),
        }
    }

    /// Perform HDC-powered semantic search.
    ///
    /// # Panics
    ///
    /// Panics if the index is internally inconsistent and a symbol is missing
    /// its cached file or symbol fingerprint. That indicates a broken
    /// `WorkspaceIndex::from_source_files_with_root` construction path.
    pub fn semantic_search(&self, query: &str, limit: u32) -> Vec<SearchResult> {
        let limit = limit as usize;
        if limit == 0 || self.symbols_by_id.is_empty() {
            return Vec::new();
        }

        let query_file = SourceFile {
            path: "<query>".to_string(),
            language: "query".to_string(),
            content: query.to_string(),
            symbols: Vec::new(),
            imports: Vec::new(),
        };
        let query_fp = fingerprint_file(&query_file);

        let mut scored = self
            .symbols_by_id
            .iter()
            .map(|(id, symbol)| {
                let symbol_fp = self
                    .symbol_fingerprints
                    .get(id)
                    .expect("symbol fingerprint missing");
                let file_fp = self
                    .file_fingerprints
                    .get(&id.file_path)
                    .expect("file fingerprint missing");
                let score = 0.7_f64.mul_add(
                    similarity(&query_fp, symbol_fp),
                    0.3 * similarity(&query_fp, file_fp),
                );
                SearchResult {
                    score,
                    symbol: symbol.clone(),
                }
            })
            .collect::<Vec<_>>();

        sort_search_results(&mut scored);
        scored.truncate(limit);
        scored
    }

    /// Return a file-level structural overview for one indexed file.
    ///
    /// # Errors
    ///
    /// Returns an error if the file is not present in the index.
    pub fn file_ast_for(&self, file: &str) -> Result<FileAst> {
        let key = self.resolve_file_key(file)?;
        let source = self
            .files_by_path
            .get(&key)
            .ok_or_else(|| anyhow!("file '{file}' was not indexed"))?;
        Ok(FileAst {
            file_path: key.clone(),
            language: source.language.clone(),
            imports: source.imports.clone(),
            symbols: self.symbols_in_file(&key),
            line_count: source.content.lines().count(),
        })
    }

    /// Return high-level statistics for the in-memory index.
    pub fn stats(&self) -> IndexStats {
        let mut languages = HashMap::new();
        for source in self.files_by_path.values() {
            *languages.entry(source.language.clone()).or_insert(0) += 1;
        }

        let mut top_symbols = self
            .symbols_by_id
            .values()
            .cloned()
            .map(|symbol| SearchResult {
                score: self.pagerank(&symbol.id),
                symbol,
            })
            .collect::<Vec<_>>();
        sort_search_results(&mut top_symbols);
        top_symbols.truncate(10);

        IndexStats {
            indexed_files: self.file_paths.len(),
            total_symbols: self.symbols_by_id.len(),
            total_edges: self.graph.edge_count(),
            edge_breakdown: HashMap::from([
                (
                    "imports".to_string(),
                    self.graph.edge_count_by_kind(EdgeKind::Imports),
                ),
                (
                    "calls".to_string(),
                    self.graph.edge_count_by_kind(EdgeKind::Calls),
                ),
                (
                    "implements".to_string(),
                    self.graph.edge_count_by_kind(EdgeKind::Implements),
                ),
                (
                    "contains".to_string(),
                    self.graph.edge_count_by_kind(EdgeKind::Contains),
                ),
                (
                    "type_ref".to_string(),
                    self.graph.edge_count_by_kind(EdgeKind::TypeRef),
                ),
            ]),
            languages,
            top_symbols_by_pagerank: top_symbols,
        }
    }

    /// Find reference-like graph locations for a symbol.
    ///
    /// # Errors
    ///
    /// Returns an error if `file_path` is provided and cannot be resolved.
    pub fn references_for_symbol(
        &self,
        symbol_name: &str,
        file_path: Option<&str>,
        include_definitions: bool,
    ) -> Result<Vec<ReferenceMatch>> {
        let mut references = Vec::new();
        for symbol in self.resolve_symbols(symbol_name, file_path)? {
            if include_definitions {
                references.push(ReferenceMatch {
                    kind: ReferenceKind::Definition,
                    location: SymbolRef::new(symbol.id.file_path.clone(), symbol.line, 0),
                    symbol: symbol.clone(),
                });
            }
            for kind in [
                EdgeKind::Imports,
                EdgeKind::Calls,
                EdgeKind::Implements,
                EdgeKind::Contains,
                EdgeKind::TypeRef,
            ] {
                references.extend(self.references_for_kind(&symbol.id, kind));
            }
        }

        references.sort_by(|left, right| {
            left.location
                .file
                .cmp(&right.location.file)
                .then_with(|| left.location.line.cmp(&right.location.line))
                .then_with(|| left.symbol.id.symbol_name.cmp(&right.symbol.id.symbol_name))
        });
        references.dedup();
        Ok(references)
    }

    /// Return impl blocks that match the requested trait.
    pub fn implementations_for_trait(&self, trait_name: &str) -> Vec<ImplementationMatch> {
        let mut matches = self
            .symbols_by_id
            .values()
            .filter(|symbol| symbol.id.kind == SymbolKind::Impl)
            .filter_map(|symbol| {
                parse_impl_symbol_name(&symbol.id.symbol_name)
                    .map(|(trt, implementor)| (symbol, trt.to_string(), implementor.to_string()))
            })
            .filter(|(_, trt, _)| trt == trait_name)
            .map(|(symbol, trt, implementor)| ImplementationMatch {
                trait_name: trt,
                implementor,
                implementation: symbol.clone(),
                methods: self.impl_methods(&symbol.id),
            })
            .collect::<Vec<_>>();

        matches.sort_by(|left, right| {
            left.implementation
                .id
                .file_path
                .cmp(&right.implementation.id.file_path)
                .then_with(|| left.implementor.cmp(&right.implementor))
        });
        matches
    }

    /// Produce a simple workspace map grouped by file.
    pub fn map_workspace(&self, focus: Option<&str>) -> WorkspaceMap {
        let mut files = self
            .files_by_path
            .iter()
            .filter(|(path, _)| focus.is_none_or(|focus| path.contains(focus)))
            .map(|(path, source)| WorkspaceMapEntry {
                file_path: path.clone(),
                language: source.language.clone(),
                symbol_count: source.symbols.len(),
                symbols: source
                    .symbols
                    .iter()
                    .take(8)
                    .map(|symbol| symbol.name.clone())
                    .collect(),
            })
            .collect::<Vec<_>>();

        files.sort_by(|left, right| left.file_path.cmp(&right.file_path));
        WorkspaceMap {
            root: normalize_workspace_path(&self.root),
            files,
        }
    }

    /// Assemble a budgeted context window from search hits.
    pub fn context_for_query(
        &self,
        query: &str,
        max_results: usize,
        token_budget: usize,
        overlay: Option<&ContextOverlay>,
        privacy: Option<&PrivacyConfig>,
    ) -> AssembledContext {
        let keyword_results = self.keyword_search(
            &KeywordQuery {
                text: query.to_string(),
                scope: SearchScope::Both,
                case_sensitive: false,
                whole_word: false,
            },
            max_results.max(1),
        );
        let semantic_limit = u32::try_from(max_results.max(1)).unwrap_or(u32::MAX);
        let semantic_results = self.semantic_search(query, semantic_limit);
        let mut candidates = keyword_results
            .into_iter()
            .chain(semantic_results)
            .map(|result| result.symbol)
            .collect::<Vec<_>>();
        sort_symbol_infos(&mut candidates);
        candidates.dedup();

        let max_depth = overlay.map_or(1, |overlay| overlay.max_expansion_depth.max(1));
        let mut slices = Vec::new();
        let mut total_tokens = 0usize;
        let mut truncated = false;

        'symbols: for symbol in candidates {
            if is_excluded_symbol(&symbol, privacy)
                || is_excluded_file(&symbol.id.file_path, overlay, privacy)
            {
                continue;
            }

            let mut related_ids = vec![symbol.id.clone()];
            if max_depth > 1 {
                related_ids.extend(
                    self.related_symbols(&symbol.id, max_depth - 1, false)
                        .into_iter()
                        .map(|info| info.id),
                );
            }

            for symbol_id in related_ids {
                if let Some(mut slice) = self.slice_for_symbol(&symbol_id) {
                    if is_excluded_file(&slice.file_path, overlay, privacy) {
                        continue;
                    }
                    apply_privacy(&mut slice, privacy);
                    if slice.token_estimate == 0 {
                        continue;
                    }
                    if total_tokens + slice.token_estimate > token_budget {
                        truncated = true;
                        break 'symbols;
                    }
                    total_tokens += slice.token_estimate;
                    slices.push(slice);
                }
            }
        }

        slices.sort_by(|left, right| {
            left.file_path
                .cmp(&right.file_path)
                .then_with(|| left.start_line.cmp(&right.start_line))
        });
        slices.dedup();

        AssembledContext {
            query: query.to_string(),
            slices,
            token_estimate: total_tokens,
            truncated,
        }
    }

    /// Workspace root used for path resolution.
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn from_source_files_with_root(root: PathBuf, files: Vec<SourceFile>) -> Self {
        let graph = build_graph(&files);
        let pagerank_scores = pagerank(&graph, 30, 0.85);
        let mut files_by_path = HashMap::new();
        let mut file_paths = HashSet::new();
        let mut imports_by_file = HashMap::new();
        let mut symbols_by_name: HashMap<String, Vec<SymbolInfo>> = HashMap::new();
        let mut functions_by_name: HashMap<String, Vec<SymbolInfo>> = HashMap::new();
        let mut symbols_by_id = HashMap::new();
        let mut file_fingerprints = HashMap::new();
        let mut symbol_fingerprints = HashMap::new();

        for file in &files {
            let key = normalize_workspace_path(&file.path);
            files_by_path.insert(key.clone(), file.clone());
            file_paths.insert(key.clone());
            imports_by_file.insert(key.clone(), file.imports.clone());

            let file_fp = fingerprint_file(file);
            file_fingerprints.insert(key.clone(), file_fp);

            for symbol in &file.symbols {
                let id = SymbolId::from_symbol(symbol, &key);
                let info = SymbolInfo {
                    id: id.clone(),
                    visibility: symbol.visibility.clone(),
                    line: symbol.line,
                    language: file.language.clone(),
                };

                symbols_by_name
                    .entry(symbol.name.clone())
                    .or_default()
                    .push(info.clone());
                if symbol.kind == SymbolKind::Function {
                    functions_by_name
                        .entry(symbol.name.clone())
                        .or_default()
                        .push(info.clone());
                }
                symbols_by_id.insert(id.clone(), info);

                let symbol_fp = fingerprint_symbol(symbol, file.content.as_bytes());
                symbol_fingerprints.insert(id, symbol_fp);
            }
        }

        sort_symbol_lists(&mut symbols_by_name);
        sort_symbol_lists(&mut functions_by_name);

        Self {
            root,
            files_by_path,
            file_paths,
            imports_by_file,
            symbols_by_name,
            functions_by_name,
            symbols_by_id,
            file_fingerprints,
            symbol_fingerprints,
            pagerank_scores,
            graph,
        }
    }

    fn resolve_file_key(&self, file: &str) -> Result<String> {
        let normalized = normalize_workspace_path(file);
        if self.file_paths.contains(&normalized) {
            return Ok(normalized);
        }

        let candidate = Path::new(file);
        let absolute = if candidate.is_absolute() {
            candidate.to_path_buf()
        } else {
            self.root.join(candidate)
        };

        let canonical = std::fs::canonicalize(&absolute)
            .with_context(|| format!("resolve workspace file {file}"))?;
        let relative = canonical
            .strip_prefix(&self.root)
            .map_err(|_| anyhow!("file '{file}' is outside workspace"))?;
        let key = normalize_workspace_path(relative);
        if self.file_paths.contains(&key) {
            return Ok(key);
        }

        bail!("file '{file}' was not indexed");
    }

    fn symbols_in_file(&self, file_path: &str) -> Vec<SymbolInfo> {
        let mut symbols = self
            .symbols_by_id
            .values()
            .filter(|symbol| symbol.id.file_path == file_path)
            .cloned()
            .collect::<Vec<_>>();
        sort_symbol_infos(&mut symbols);
        symbols
    }

    fn pagerank(&self, id: &SymbolId) -> f64 {
        self.pagerank_scores.get(id).copied().unwrap_or(0.0)
    }

    fn resolve_symbols(
        &self,
        symbol_name: &str,
        file_path: Option<&str>,
    ) -> Result<Vec<SymbolInfo>> {
        let mut matches = self.symbol_lookup(symbol_name);
        if let Some(file_path) = file_path {
            let key = self.resolve_file_key(file_path)?;
            matches.retain(|symbol| symbol.id.file_path == key);
        }
        Ok(matches)
    }

    fn related_symbols(&self, id: &SymbolId, depth: usize, reverse: bool) -> Vec<SymbolInfo> {
        if depth == 0 {
            return Vec::new();
        }

        let ids = if reverse {
            breadth_first_reverse(&self.graph, id, depth)
        } else {
            self.graph.transitive(id, depth)
        };

        let mut symbols = ids
            .into_iter()
            .filter_map(|(related, _)| self.symbols_by_id.get(&related).cloned())
            .collect::<Vec<_>>();
        sort_symbol_infos(&mut symbols);
        symbols
    }

    fn references_for_kind(&self, id: &SymbolId, kind: EdgeKind) -> Vec<ReferenceMatch> {
        self.graph
            .reverse_neighbors_by_kind(id, kind.clone())
            .into_iter()
            .filter_map(|source| self.symbols_by_id.get(source).cloned())
            .map(|symbol| ReferenceMatch {
                kind: reference_kind(&kind),
                location: SymbolRef::new(symbol.id.file_path.clone(), symbol.line, 0),
                symbol,
            })
            .collect()
    }

    fn impl_methods(&self, impl_id: &SymbolId) -> Vec<SymbolInfo> {
        let Some(source) = self.files_by_path.get(&impl_id.file_path) else {
            return Vec::new();
        };
        let Some(impl_symbol) = source
            .symbols
            .iter()
            .find(|symbol| SymbolId::from_symbol(symbol, &impl_id.file_path) == *impl_id)
        else {
            return Vec::new();
        };
        let next_impl_line = source
            .symbols
            .iter()
            .filter(|symbol| symbol.kind == SymbolKind::Impl && symbol.line > impl_symbol.line)
            .map(|symbol| symbol.line)
            .min()
            .unwrap_or(usize::MAX);
        let mut methods = source
            .symbols
            .iter()
            .filter(|symbol| {
                symbol.kind == SymbolKind::Function
                    && symbol.line > impl_symbol.line
                    && symbol.line < next_impl_line
            })
            .filter_map(|symbol| {
                self.symbols_by_id
                    .get(&SymbolId::from_symbol(symbol, &impl_id.file_path))
                    .cloned()
            })
            .collect::<Vec<_>>();
        sort_symbol_infos(&mut methods);
        methods
    }

    fn slice_for_symbol(&self, id: &SymbolId) -> Option<CodeSlice> {
        let source = self.files_by_path.get(&id.file_path)?;
        let mut symbols = source.symbols.iter().collect::<Vec<_>>();
        symbols.sort_by_key(|symbol| symbol.line);
        let (index, symbol) = symbols
            .iter()
            .enumerate()
            .find(|(_, symbol)| SymbolId::from_symbol(symbol, &id.file_path) == *id)?;
        let start_line = symbol.line.max(1);
        let end_line = symbols
            .get(index + 1)
            .map(|next| next.line.saturating_sub(1))
            .unwrap_or_else(|| source.content.lines().count().max(start_line));
        let content = source
            .content
            .lines()
            .enumerate()
            .skip(start_line.saturating_sub(1))
            .take(end_line.saturating_sub(start_line).saturating_add(1))
            .map(|(_, line)| line)
            .collect::<Vec<_>>()
            .join("\n");

        Some(CodeSlice {
            file_path: id.file_path.clone(),
            start_line,
            end_line,
            token_estimate: estimate_tokens(&content),
            content,
            symbols_included: vec![id.clone()],
        })
    }

    fn traverse_call_graph(
        &self,
        roots: &[SymbolInfo],
        depth: usize,
        direction: CallGraphDirection,
    ) -> (Vec<SymbolInfo>, Vec<CallGraphEdge>) {
        if depth == 0 {
            return (Vec::new(), Vec::new());
        }

        let mut visited: HashSet<SymbolId> = roots.iter().map(|info| info.id.clone()).collect();
        let mut queue: VecDeque<(SymbolId, usize)> =
            roots.iter().map(|info| (info.id.clone(), 0)).collect();
        let mut nodes = Vec::new();
        let mut edges = Vec::new();
        let mut seen_edges = HashSet::new();

        while let Some((current_id, current_depth)) = queue.pop_front() {
            if current_depth >= depth {
                continue;
            }

            let neighbors = match direction {
                CallGraphDirection::Callees => {
                    self.graph.neighbors_by_kind(&current_id, EdgeKind::Calls)
                }
                CallGraphDirection::Callers => self
                    .graph
                    .reverse_neighbors_by_kind(&current_id, EdgeKind::Calls),
            };

            let Some(from_info) = self.symbols_by_id.get(&current_id).cloned() else {
                continue;
            };

            for neighbor in neighbors {
                let neighbor_id = (*neighbor).clone();
                let Some(to_info) = self.symbols_by_id.get(&neighbor_id).cloned() else {
                    continue;
                };

                let direction_key = match direction {
                    CallGraphDirection::Callees => 0_u8,
                    CallGraphDirection::Callers => 1_u8,
                };
                let edge_key = (
                    from_info.id.clone(),
                    to_info.id.clone(),
                    direction_key,
                    current_depth + 1,
                );
                if seen_edges.insert(edge_key) {
                    edges.push(CallGraphEdge {
                        from: from_info.clone(),
                        to: to_info.clone(),
                        direction,
                        depth: current_depth + 1,
                    });
                }

                if visited.insert(neighbor_id.clone()) {
                    nodes.push(to_info);
                    queue.push_back((neighbor_id, current_depth + 1));
                }
            }
        }

        sort_symbol_infos(&mut nodes);
        edges.sort_by(|left, right| {
            left.depth
                .cmp(&right.depth)
                .then_with(|| left.from.id.file_path.cmp(&right.from.id.file_path))
                .then_with(|| left.from.id.symbol_name.cmp(&right.from.id.symbol_name))
                .then_with(|| left.to.id.file_path.cmp(&right.to.id.file_path))
                .then_with(|| left.to.id.symbol_name.cmp(&right.to.id.symbol_name))
        });
        (nodes, edges)
    }
}

fn collect_source_files(root: &Path) -> Result<Vec<SourceFile>> {
    let mut files = Vec::new();
    collect_source_files_in_dir(root, root, &mut files)?;
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(files)
}

fn collect_source_files_in_dir(root: &Path, dir: &Path, out: &mut Vec<SourceFile>) -> Result<()> {
    for entry in std::fs::read_dir(dir).with_context(|| format!("read {}", dir.display()))? {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                tracing::debug!(error = %err, "skip unreadable workspace entry");
                continue;
            }
        };

        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(err) => {
                tracing::debug!(error = %err, path = %path.display(), "skip unreadable file type");
                continue;
            }
        };

        if file_type.is_dir() {
            if should_skip_dir(&path) {
                continue;
            }
            collect_source_files_in_dir(root, &path, out)?;
            continue;
        }

        if !file_type.is_file() {
            continue;
        }

        let Some(provider) = provider_for_path(&path) else {
            continue;
        };

        let content = match std::fs::read_to_string(&path) {
            Ok(content) => content,
            Err(err) => {
                tracing::debug!(error = %err, path = %path.display(), "skip unreadable source file");
                continue;
            }
        };

        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        out.push(parse_source(&relative, &content, provider));
    }

    Ok(())
}

fn provider_for_path(path: &Path) -> Option<&'static dyn LanguageProvider> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "rs" => Some(&RUST_PROVIDER),
        "ts" | "tsx" | "js" | "jsx" => Some(&TS_PROVIDER),
        "go" => Some(&GO_PROVIDER),
        _ => None,
    }
}

fn should_skip_dir(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };

    matches!(
        name,
        ".git"
            | ".roko"
            | "target"
            | "node_modules"
            | "dist"
            | "build"
            | "coverage"
            | ".next"
            | ".turbo"
            | ".cache"
    )
}

fn normalize_workspace_path(path: impl AsRef<Path>) -> String {
    path.as_ref().to_string_lossy().replace('\\', "/")
}

fn normalize_match_field(value: &str, case_sensitive: bool) -> String {
    if case_sensitive {
        value.to_string()
    } else {
        value.to_ascii_lowercase()
    }
}

fn keyword_matches(haystack: &str, needle: &str, whole_word: bool) -> bool {
    if needle.is_empty() {
        return false;
    }
    if whole_word {
        haystack == needle
    } else {
        haystack.contains(needle)
    }
}

/// Reciprocal Rank Fusion: merge multiple ranked lists into one.
///
/// RRF(d) = sum(1 / (k + rank_i(d))) across all lists that contain `d`.
/// `k` is typically 60. Results are deduplicated by `SymbolId` and sorted by
/// descending RRF score.
fn rrf_merge(lists: &[Vec<SearchResult>], k: f64, limit: usize) -> Vec<SearchResult> {
    use std::collections::hash_map::Entry;

    let mut scores: HashMap<SymbolId, (f64, SearchResult)> = HashMap::new();
    for list in lists {
        for (rank, result) in list.iter().enumerate() {
            let rrf_score = 1.0 / (k + (rank + 1) as f64);
            match scores.entry(result.symbol.id.clone()) {
                Entry::Occupied(mut entry) => {
                    entry.get_mut().0 += rrf_score;
                }
                Entry::Vacant(entry) => {
                    entry.insert((rrf_score, result.clone()));
                }
            }
        }
    }

    let mut merged: Vec<SearchResult> = scores
        .into_values()
        .map(|(rrf_score, mut result)| {
            result.score = rrf_score;
            result
        })
        .collect();

    sort_search_results(&mut merged);
    merged.truncate(limit);
    merged
}

fn matches_file_pattern(path: &str, pattern: &str) -> bool {
    if pattern.is_empty() || pattern == "*" || pattern == "**" {
        return true;
    }

    let normalized = pattern.replace("**", "*");
    let parts = normalized
        .split('*')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if parts.is_empty() {
        return true;
    }

    let mut cursor = 0usize;
    for part in parts {
        let Some(found) = path[cursor..].find(part) else {
            return false;
        };
        cursor += found + part.len();
    }
    true
}

fn sort_symbol_lists(map: &mut HashMap<String, Vec<SymbolInfo>>) {
    for values in map.values_mut() {
        sort_symbol_infos(values);
    }
}

fn compare_search_results(left: &SearchResult, right: &SearchResult) -> std::cmp::Ordering {
    right
        .score
        .total_cmp(&left.score)
        .then_with(|| left.symbol.id.file_path.cmp(&right.symbol.id.file_path))
        .then_with(|| left.symbol.id.symbol_name.cmp(&right.symbol.id.symbol_name))
        .then_with(|| left.symbol.line.cmp(&right.symbol.line))
}

fn sort_search_results(results: &mut [SearchResult]) {
    results.sort_by(compare_search_results);
}

fn sort_symbol_infos(values: &mut [SymbolInfo]) {
    values.sort_by(|left, right| {
        left.id
            .file_path
            .cmp(&right.id.file_path)
            .then_with(|| left.id.symbol_name.cmp(&right.id.symbol_name))
            .then_with(|| left.line.cmp(&right.line))
    });
}

fn breadth_first_reverse(
    graph: &SymbolGraph,
    start: &SymbolId,
    max_depth: usize,
) -> Vec<(SymbolId, usize)> {
    if max_depth == 0 {
        return Vec::new();
    }

    let mut visited = HashSet::from([start.clone()]);
    let mut queue = VecDeque::from([(start.clone(), 0usize)]);
    let mut results = Vec::new();

    while let Some((node, depth)) = queue.pop_front() {
        if depth >= max_depth {
            continue;
        }
        for neighbor in graph.reverse_neighbors(&node) {
            if visited.insert(neighbor.clone()) {
                let next_depth = depth + 1;
                results.push((neighbor.clone(), next_depth));
                queue.push_back((neighbor.clone(), next_depth));
            }
        }
    }

    results
}

fn estimate_tokens(content: &str) -> usize {
    content.chars().count().div_ceil(4)
}

const fn reference_kind(kind: &EdgeKind) -> ReferenceKind {
    match kind {
        EdgeKind::Calls => ReferenceKind::Calls,
        EdgeKind::Imports => ReferenceKind::Imports,
        EdgeKind::Implements => ReferenceKind::Implements,
        EdgeKind::Contains => ReferenceKind::Contains,
        EdgeKind::TypeRef => ReferenceKind::TypeRef,
    }
}

fn parse_impl_symbol_name(name: &str) -> Option<(&str, &str)> {
    let (trait_name, implementor) = name.split_once(" for ")?;
    Some((trait_name, implementor))
}

fn is_excluded_symbol(symbol: &SymbolInfo, privacy: Option<&PrivacyConfig>) -> bool {
    privacy.is_some_and(|privacy| {
        privacy
            .blocked_symbols
            .iter()
            .any(|blocked| blocked == &symbol.id.symbol_name)
    })
}

fn is_excluded_file(
    file_path: &str,
    overlay: Option<&ContextOverlay>,
    privacy: Option<&PrivacyConfig>,
) -> bool {
    if overlay.is_some_and(|overlay| {
        overlay
            .excluded_patterns
            .iter()
            .any(|pattern| matches_file_pattern(file_path, pattern))
    }) {
        return true;
    }

    privacy.is_some_and(|privacy| {
        privacy
            .ignore_files
            .iter()
            .any(|ignored| matches_file_pattern(file_path, ignored))
    })
}

fn apply_privacy(slice: &mut CodeSlice, privacy: Option<&PrivacyConfig>) {
    let Some(privacy) = privacy else {
        return;
    };

    for pattern in &privacy.redact_patterns {
        if pattern.is_empty() {
            continue;
        }
        slice.content = slice.content.replace(pattern, "[REDACTED]");
    }
    slice.token_estimate = estimate_tokens(&slice.content);
}

impl CodeIndex for WorkspaceIndex {
    fn lookup_symbol(&self, name: &str) -> Vec<SymbolInfo> {
        self.symbol_lookup(name)
    }

    fn search_by_keyword(&self, query: &KeywordQuery, limit: usize) -> Vec<SearchResult> {
        self.keyword_search(query, limit)
    }

    fn search_by_structure(&self, query: &StructuralQuery, limit: usize) -> Vec<SearchResult> {
        self.structural_search(query, limit)
    }

    fn search_by_fingerprint(&self, query: &HdcQuery) -> Vec<SearchResult> {
        self.hdc_search(query)
    }

    fn search_by_embedding(&self, query: &EmbeddingQuery) -> Vec<SearchResult> {
        self.embedding_search(query)
    }

    fn list_imports_for_file(&self, file: &str) -> Result<Vec<Import>> {
        self.imports_for_file(file)
    }

    fn build_symbol_context(
        &self,
        symbol_name: &str,
        file_path: Option<&str>,
        expansion_depth: usize,
    ) -> Result<Vec<SymbolContext>> {
        self.symbol_context(symbol_name, file_path, expansion_depth)
    }

    fn find_call_graph(&self, function: &str, depth: u32) -> CallGraph {
        self.call_graph(function, depth)
    }

    fn file_ast(&self, file: &str) -> Result<FileAst> {
        self.file_ast_for(file)
    }

    fn index_stats(&self) -> IndexStats {
        self.stats()
    }

    fn find_references(
        &self,
        symbol_name: &str,
        file_path: Option<&str>,
        include_definitions: bool,
    ) -> Result<Vec<ReferenceMatch>> {
        self.references_for_symbol(symbol_name, file_path, include_definitions)
    }

    fn find_implementations(&self, trait_name: &str) -> Vec<ImplementationMatch> {
        self.implementations_for_trait(trait_name)
    }

    fn workspace_map(&self, focus: Option<&str>) -> WorkspaceMap {
        self.map_workspace(focus)
    }

    fn assemble_context(
        &self,
        query: &str,
        max_results: usize,
        token_budget: usize,
        overlay: Option<&ContextOverlay>,
        privacy: Option<&PrivacyConfig>,
    ) -> AssembledContext {
        self.context_for_query(query, max_results, token_budget, overlay, privacy)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::language::{ImportKind, Symbol, SymbolKind, Visibility};

    fn symbol(name: &str, kind: SymbolKind, line: usize) -> Symbol {
        Symbol {
            name: name.to_string(),
            kind,
            visibility: Visibility::Public,
            line,
        }
    }

    fn file(
        path: &str,
        language: &str,
        content: &str,
        symbols: Vec<Symbol>,
        imports: Vec<Import>,
    ) -> SourceFile {
        SourceFile {
            path: path.to_string(),
            language: language.to_string(),
            content: content.to_string(),
            symbols,
            imports,
        }
    }

    #[test]
    fn symbol_lookup_returns_all_matches() {
        let index = WorkspaceIndex::from_source_files(vec![
            file(
                "a.rs",
                "rust",
                "fn foo() {}\n",
                vec![symbol("foo", SymbolKind::Function, 1)],
                vec![],
            ),
            file(
                "b.rs",
                "rust",
                "fn foo() {}\n",
                vec![symbol("foo", SymbolKind::Function, 1)],
                vec![],
            ),
        ]);

        let results = index.symbol_lookup("foo");
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].id.file_path, "a.rs");
        assert_eq!(results[1].id.file_path, "b.rs");
    }

    #[test]
    fn imports_for_file_returns_imports() {
        let index = WorkspaceIndex::from_source_files(vec![file(
            "lib.rs",
            "rust",
            "use crate::thing::Thing;\n",
            vec![symbol("thing", SymbolKind::Function, 1)],
            vec![Import {
                path: "crate::thing::Thing".to_string(),
                alias: None,
                kind: ImportKind::Use,
            }],
        )]);

        let imports = index.imports_for_file("lib.rs").expect("imports");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].path, "crate::thing::Thing");
    }

    #[test]
    fn call_graph_finds_callers_and_callees() {
        let index = WorkspaceIndex::from_source_files(vec![file(
            "a.rs",
            "rust",
            "fn helper() {}\nfn main() { helper(); }\n",
            vec![
                symbol("helper", SymbolKind::Function, 1),
                symbol("main", SymbolKind::Function, 2),
            ],
            vec![],
        )]);

        let graph = index.call_graph("main", 2);
        assert_eq!(graph.roots.len(), 1);
        assert_eq!(graph.callees.len(), 1);
        assert_eq!(graph.callees[0].id.symbol_name, "helper");
        assert!(graph.callers.is_empty());
    }

    #[test]
    fn semantic_search_returns_ranked_hits() {
        let index = WorkspaceIndex::from_source_files(vec![
            file(
                "a.rs",
                "rust",
                "fn alpha() {}\n",
                vec![symbol("alpha", SymbolKind::Function, 1)],
                vec![],
            ),
            file(
                "b.rs",
                "rust",
                "fn beta() {}\n",
                vec![symbol("beta", SymbolKind::Function, 1)],
                vec![],
            ),
        ]);

        let results = index.semantic_search("alpha", 1);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].symbol.id.symbol_name, "alpha");
    }

    fn make_result(name: &str, file: &str, score: f64) -> SearchResult {
        SearchResult {
            score,
            symbol: SymbolInfo {
                id: SymbolId {
                    file_path: file.to_string(),
                    symbol_name: name.to_string(),
                    kind: SymbolKind::Function,
                },
                visibility: Visibility::Public,
                line: 1,
                language: "rust".to_string(),
            },
        }
    }

    #[test]
    fn rrf_merge_combines_two_lists() {
        let list_a = vec![
            make_result("alpha", "a.rs", 1.0),
            make_result("beta", "b.rs", 0.9),
        ];
        let list_b = vec![
            make_result("beta", "b.rs", 1.0),
            make_result("gamma", "c.rs", 0.8),
        ];

        let merged = rrf_merge(&[list_a, list_b], 60.0, 10);
        assert_eq!(merged.len(), 3);
        // beta appears in both lists so should have the highest RRF score.
        assert_eq!(merged[0].symbol.id.symbol_name, "beta");
        // RRF(beta) = 1/(60+2) + 1/(60+1) = 1/62 + 1/61
        let expected_beta = 1.0 / 62.0 + 1.0 / 61.0;
        assert!((merged[0].score - expected_beta).abs() < 1e-10);
    }

    #[test]
    fn rrf_merge_respects_limit() {
        let list = vec![
            make_result("a", "a.rs", 1.0),
            make_result("b", "b.rs", 0.9),
            make_result("c", "c.rs", 0.8),
        ];
        let merged = rrf_merge(&[list], 60.0, 2);
        assert_eq!(merged.len(), 2);
    }

    #[test]
    fn rrf_merge_empty_lists() {
        let merged = rrf_merge(&[], 60.0, 10);
        assert!(merged.is_empty());
    }

    #[test]
    fn unified_search_keyword() {
        let index = WorkspaceIndex::from_source_files(vec![file(
            "a.rs",
            "rust",
            "fn hello() {}\n",
            vec![symbol("hello", SymbolKind::Function, 1)],
            vec![],
        )]);

        let results = index.search(
            SearchStrategy::Keyword(KeywordQuery {
                text: "hello".to_string(),
                scope: SearchScope::Symbols,
                case_sensitive: false,
                whole_word: false,
            }),
            10,
        );
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].symbol.id.symbol_name, "hello");
    }

    #[test]
    fn unified_search_hybrid() {
        let index = WorkspaceIndex::from_source_files(vec![
            file(
                "a.rs",
                "rust",
                "fn alpha() {}\n",
                vec![symbol("alpha", SymbolKind::Function, 1)],
                vec![],
            ),
            file(
                "b.rs",
                "rust",
                "fn beta() {}\n",
                vec![symbol("beta", SymbolKind::Function, 1)],
                vec![],
            ),
        ]);

        let results = index.search(
            SearchStrategy::Hybrid {
                keyword: Some(KeywordQuery {
                    text: "alpha".to_string(),
                    scope: SearchScope::Symbols,
                    case_sensitive: false,
                    whole_word: false,
                }),
                structural: None,
                hdc: None,
            },
            10,
        );
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].symbol.id.symbol_name, "alpha");
    }
}
