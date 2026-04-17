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

use crate::graph::{EdgeKind, SymbolGraph, build_graph};
use crate::hdc::{fingerprint_file, fingerprint_symbol, similarity};
use crate::parser::{SourceFile, parse_source};
use crate::symbol::SymbolId;

static RUST_PROVIDER: RustLanguageProvider = RustLanguageProvider;
static TS_PROVIDER: TypeScriptLanguageProvider = TypeScriptLanguageProvider;
static GO_PROVIDER: GoLanguageProvider = GoLanguageProvider;

/// Serializable symbol summary for MCP responses.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

/// Workspace-wide code index.
#[derive(Clone, Debug)]
pub struct WorkspaceIndex {
    root: PathBuf,
    file_paths: HashSet<String>,
    imports_by_file: HashMap<String, Vec<Import>>,
    symbols_by_name: HashMap<String, Vec<SymbolInfo>>,
    functions_by_name: HashMap<String, Vec<SymbolInfo>>,
    symbols_by_id: HashMap<SymbolId, SymbolInfo>,
    file_fingerprints: HashMap<String, crate::hdc::HdcFingerprint>,
    symbol_fingerprints: HashMap<SymbolId, crate::hdc::HdcFingerprint>,
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

        scored.sort_by(|left, right| {
            right
                .score
                .total_cmp(&left.score)
                .then_with(|| left.symbol.id.file_path.cmp(&right.symbol.id.file_path))
                .then_with(|| left.symbol.id.symbol_name.cmp(&right.symbol.id.symbol_name))
                .then_with(|| left.symbol.line.cmp(&right.symbol.line))
        });
        scored.truncate(limit);
        scored
    }

    /// Workspace root used for path resolution.
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn from_source_files_with_root(root: PathBuf, files: Vec<SourceFile>) -> Self {
        let graph = build_graph(&files);
        let mut file_paths = HashSet::new();
        let mut imports_by_file = HashMap::new();
        let mut symbols_by_name: HashMap<String, Vec<SymbolInfo>> = HashMap::new();
        let mut functions_by_name: HashMap<String, Vec<SymbolInfo>> = HashMap::new();
        let mut symbols_by_id = HashMap::new();
        let mut file_fingerprints = HashMap::new();
        let mut symbol_fingerprints = HashMap::new();

        for file in &files {
            let key = normalize_workspace_path(&file.path);
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
            file_paths,
            imports_by_file,
            symbols_by_name,
            functions_by_name,
            symbols_by_id,
            file_fingerprints,
            symbol_fingerprints,
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

fn sort_symbol_lists(map: &mut HashMap<String, Vec<SymbolInfo>>) {
    for values in map.values_mut() {
        sort_symbol_infos(values);
    }
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
}
