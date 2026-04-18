//! MCP server for code-intelligence queries backed by `roko-index`.

use anyhow::Result;
use roko_index::WorkspaceIndex;
use roko_mcp_stdio::{JsonRpcError, JsonRpcRequest, serve_stdio};
use serde::Deserialize;
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

#[derive(Debug, Deserialize)]
struct ToolsCallParams {
    name: String,
    #[serde(default = "empty_json_object")]
    arguments: Value,
}

#[derive(Debug, Deserialize)]
struct SymbolLookupArguments {
    name: String,
}

#[derive(Debug, Deserialize)]
struct CallGraphArguments {
    function: String,
    depth: u32,
}

#[derive(Debug, Deserialize)]
struct ImportsArguments {
    file: String,
}

#[derive(Debug, Deserialize)]
struct SemanticSearchArguments {
    query: String,
    limit: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum SearchStrategy {
    Keyword,
    Structural,
    Hdc,
    Embedding,
    Hybrid,
}

#[derive(Debug, Deserialize)]
struct SearchCodeArguments {
    query: String,
    #[serde(default = "default_search_strategy")]
    strategy: SearchStrategy,
    #[serde(default = "default_max_results")]
    max_results: u32,
    #[serde(default)]
    file_pattern: Option<String>,
    #[serde(default)]
    kind_filter: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GetSymbolContextArguments {
    symbol_name: String,
    #[serde(default)]
    file_path: Option<String>,
    #[serde(default = "default_true")]
    include_dependencies: bool,
    #[serde(default = "default_true")]
    include_callers: bool,
    #[serde(default = "default_expansion_depth")]
    expansion_depth: u32,
}

#[derive(Debug, Deserialize)]
struct GetFileAstArguments {
    file_path: String,
    #[serde(default)]
    include_bodies: bool,
}

#[derive(Debug, Deserialize)]
struct FindSimilarPatternsArguments {
    reference: String,
    #[serde(default = "default_min_similarity")]
    min_similarity: f64,
    #[serde(default = "default_max_results")]
    max_results: u32,
}

#[derive(Debug, Deserialize)]
struct FindReferencesArguments {
    symbol_name: String,
    #[serde(default)]
    file_path: Option<String>,
    #[serde(default)]
    include_definitions: bool,
}

#[derive(Debug, Deserialize)]
struct FindImplementationsArguments {
    trait_name: String,
    #[serde(default = "default_true")]
    include_methods: bool,
}

#[derive(Debug, Deserialize)]
struct GetCallersArguments {
    function_name: String,
    #[serde(default)]
    file_path: Option<String>,
    #[serde(default)]
    transitive: bool,
    #[serde(default = "default_max_depth")]
    max_depth: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum WorkspaceMapDepth {
    Crate,
    Module,
    Symbol,
}

#[derive(Debug, Deserialize)]
struct WorkspaceMapArguments {
    #[serde(default = "default_workspace_map_depth")]
    depth: WorkspaceMapDepth,
    #[serde(default)]
    focus_path: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GetContextArguments {
    task: String,
    #[serde(default = "default_token_budget")]
    token_budget: usize,
    #[serde(default)]
    include_tests: bool,
}

#[derive(Clone, Debug)]
struct SearchHit {
    symbol: String,
    kind: String,
    file: String,
    line: usize,
    score: f64,
    snippet: String,
}

#[derive(Clone, Debug)]
struct ParsedFile {
    path: String,
    language: String,
    imports: Vec<String>,
    symbols: Vec<ParsedSymbol>,
}

#[derive(Clone, Debug)]
struct ParsedSymbol {
    name: String,
    kind: String,
    visibility: String,
    line: usize,
    signature: String,
}

/// Run the MCP server over stdio.
///
/// # Errors
///
/// Returns an error if the workspace index cannot be loaded or if the stdio
/// JSON-RPC loop fails to read, handle, or write requests.
pub fn run() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("roko_mcp_code=info")
        .with_writer(io::stderr)
        .init();

    let workspace = Arc::new(load_workspace_index()?);
    serve_stdio(io::stdin().lock(), io::stdout().lock(), move |request| {
        handle_request(request, workspace.as_ref())
    })?;
    Ok(())
}

fn load_workspace_index() -> Result<WorkspaceIndex> {
    let root = env::var_os("ROKO_WORKSPACE_ROOT").map_or(env::current_dir()?, PathBuf::from);
    WorkspaceIndex::load(root)
}

fn handle_request(request: JsonRpcRequest, index: &WorkspaceIndex) -> Result<Value, JsonRpcError> {
    match request.method.as_str() {
        "initialize" => Ok(handle_initialize()),
        "tools/list" => Ok(handle_tools_list()),
        "tools/call" => handle_tools_call(request.params, index),
        _ => Err(JsonRpcError::method_not_found(&request.method)),
    }
}

fn handle_initialize() -> Value {
    json!({
        "protocolVersion": "2024-11-05",
        "capabilities": {
            "tools": {}
        },
        "serverInfo": {
            "name": "roko-mcp-code",
            "version": env!("CARGO_PKG_VERSION")
        }
    })
}

#[allow(clippy::too_many_lines)]
fn handle_tools_list() -> Value {
    json!({
        "tools": [
            tool_spec(
                "search_code",
                "Search the codebase for symbols, patterns, or code matching a query.",
                &json!({
                    "type": "object",
                    "properties": {
                        "query": {"type": "string", "description": "Natural language or code query"},
                        "strategy": {
                            "type": "string",
                            "enum": ["keyword", "structural", "hdc", "embedding", "hybrid"],
                            "default": "hybrid"
                        },
                        "max_results": {"type": "integer", "default": 10},
                        "file_pattern": {"type": "string", "description": "Glob pattern to scope search."},
                        "kind_filter": {
                            "type": "string",
                            "enum": ["function", "struct", "enum", "trait", "const", "type", "module", "impl"]
                        }
                    },
                    "required": ["query"],
                    "additionalProperties": false
                })
            ),
            tool_spec(
                "get_symbol_context",
                "Get detailed context for a symbol including definition, dependencies, and callers.",
                &json!({
                    "type": "object",
                    "properties": {
                        "symbol_name": {"type": "string"},
                        "file_path": {"type": "string"},
                        "include_dependencies": {"type": "boolean", "default": true},
                        "include_callers": {"type": "boolean", "default": true},
                        "expansion_depth": {"type": "integer", "default": 1}
                    },
                    "required": ["symbol_name"],
                    "additionalProperties": false
                })
            ),
            tool_spec(
                "get_file_ast",
                "Get the symbol-level structure of a source file.",
                &json!({
                    "type": "object",
                    "properties": {
                        "file_path": {"type": "string"},
                        "include_bodies": {"type": "boolean", "default": false}
                    },
                    "required": ["file_path"],
                    "additionalProperties": false
                })
            ),
            tool_spec(
                "find_similar_patterns",
                "Find code patterns structurally similar to a reference symbol or code snippet.",
                &json!({
                    "type": "object",
                    "properties": {
                        "reference": {"type": "string"},
                        "min_similarity": {"type": "number", "default": 0.6},
                        "max_results": {"type": "integer", "default": 10}
                    },
                    "required": ["reference"],
                    "additionalProperties": false
                })
            ),
            tool_spec(
                "get_index_stats",
                "Get statistics about the code index: file count, symbol count, edge count, and languages.",
                &json!({
                    "type": "object",
                    "properties": {},
                    "additionalProperties": false
                })
            ),
            tool_spec(
                "find_references",
                "Find all locations where a symbol is referenced.",
                &json!({
                    "type": "object",
                    "properties": {
                        "symbol_name": {"type": "string"},
                        "file_path": {"type": "string"},
                        "include_definitions": {"type": "boolean", "default": false}
                    },
                    "required": ["symbol_name"],
                    "additionalProperties": false
                })
            ),
            tool_spec(
                "find_implementations",
                "Find all types that implement a given trait or interface.",
                &json!({
                    "type": "object",
                    "properties": {
                        "trait_name": {"type": "string"},
                        "include_methods": {"type": "boolean", "default": true}
                    },
                    "required": ["trait_name"],
                    "additionalProperties": false
                })
            ),
            tool_spec(
                "get_callers",
                "Find all functions that call a given function.",
                &json!({
                    "type": "object",
                    "properties": {
                        "function_name": {"type": "string"},
                        "file_path": {"type": "string"},
                        "transitive": {"type": "boolean", "default": false},
                        "max_depth": {"type": "integer", "default": 2}
                    },
                    "required": ["function_name"],
                    "additionalProperties": false
                })
            ),
            tool_spec(
                "workspace_map",
                "Get a high-level map of the workspace: crates, modules, and top symbols.",
                &json!({
                    "type": "object",
                    "properties": {
                        "depth": {
                            "type": "string",
                            "enum": ["crate", "module", "symbol"],
                            "default": "module"
                        },
                        "focus_path": {"type": "string"}
                    },
                    "additionalProperties": false
                })
            ),
            tool_spec(
                "get_context",
                "Given a task description, automatically assemble the most relevant code context.",
                &json!({
                    "type": "object",
                    "properties": {
                        "task": {"type": "string"},
                        "token_budget": {"type": "integer", "default": 40000},
                        "include_tests": {"type": "boolean", "default": false}
                    },
                    "required": ["task"],
                    "additionalProperties": false
                })
            ),
        ]
    })
}

fn handle_tools_call(params: Value, index: &WorkspaceIndex) -> Result<Value, JsonRpcError> {
    let params: ToolsCallParams = serde_json::from_value(params)
        .map_err(|err| JsonRpcError::invalid_params(format!("invalid tools/call params: {err}")))?;
    dispatch_tool_call(&params.name, params.arguments, index)
}

fn dispatch_tool_call(
    name: &str,
    arguments: Value,
    index: &WorkspaceIndex,
) -> Result<Value, JsonRpcError> {
    match name {
        "search_code" => handle_search_code(arguments, index),
        "get_symbol_context" => handle_get_symbol_context(arguments, index),
        "get_file_ast" => handle_get_file_ast(arguments, index),
        "find_similar_patterns" => handle_find_similar_patterns(arguments, index),
        "get_index_stats" => handle_get_index_stats(index),
        "find_references" => handle_find_references(arguments, index),
        "find_implementations" => handle_find_implementations(arguments, index),
        "get_callers" => handle_get_callers(arguments, index),
        "workspace_map" => handle_workspace_map(arguments, index),
        "get_context" => handle_get_context(arguments, index),
        "symbol_lookup" => handle_symbol_lookup(arguments, index),
        "call_graph" => handle_call_graph(arguments, index),
        "imports" => handle_imports(arguments, index),
        "semantic_search" => handle_semantic_search(arguments, index),
        _ => Err(JsonRpcError::invalid_params(format!(
            "unknown tool: {name}"
        ))),
    }
}

fn handle_search_code(arguments: Value, index: &WorkspaceIndex) -> Result<Value, JsonRpcError> {
    let args: SearchCodeArguments = serde_json::from_value(arguments)
        .map_err(|err| JsonRpcError::invalid_params(format!("invalid search_code args: {err}")))?;
    let started = Instant::now();
    let strategy_used = effective_search_strategy(args.strategy);
    let mut hits = match strategy_used {
        SearchStrategy::Keyword | SearchStrategy::Structural => search_keyword_hits(index, &args)?,
        SearchStrategy::Hdc | SearchStrategy::Embedding => search_semantic_hits(index, &args),
        SearchStrategy::Hybrid => {
            let mut combined = search_keyword_hits(index, &args)?;
            combined.extend(search_semantic_hits(index, &args));
            combined
        }
    };
    let total_candidates = dedupe_and_sort_hits(&mut hits, args.max_results);
    tool_result(json!({
        "results": hits.iter().map(search_hit_json).collect::<Vec<_>>(),
        "total_candidates": total_candidates,
        "strategy_used": strategy_used,
        "elapsed_ms": started.elapsed().as_millis(),
    }))
}

fn handle_get_symbol_context(
    arguments: Value,
    index: &WorkspaceIndex,
) -> Result<Value, JsonRpcError> {
    let args: GetSymbolContextArguments = serde_json::from_value(arguments).map_err(|err| {
        JsonRpcError::invalid_params(format!("invalid get_symbol_context args: {err}"))
    })?;
    let mut matches = index.symbol_lookup(&args.symbol_name);
    if let Some(file_path) = args.file_path.as_deref() {
        matches.retain(|symbol| file_matches_filter(&symbol.id.file_path, file_path));
    }

    let include_graph = matches.len() == 1
        && (args.include_callers || args.include_dependencies)
        && lookup_symbol_details(index.root(), &matches[0]).kind == "function";

    let call_graph =
        include_graph.then(|| index.call_graph(&args.symbol_name, args.expansion_depth));
    let warning = if !include_graph && (args.include_callers || args.include_dependencies) {
        Some(
            "caller/dependency expansion is only available for a uniquely resolved function symbol"
                .to_string(),
        )
    } else {
        None
    };

    let payload_matches = matches
        .iter()
        .map(|symbol| {
            let details = lookup_symbol_details(index.root(), symbol);
            let callers = if args.include_callers {
                call_graph
                    .as_ref()
                    .map(|graph| {
                        graph
                            .callers
                            .iter()
                            .map(symbol_summary_json)
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            let dependencies = if args.include_dependencies {
                call_graph
                    .as_ref()
                    .map(|graph| {
                        graph
                            .callees
                            .iter()
                            .map(symbol_summary_json)
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default()
            } else {
                Vec::new()
            };

            json!({
                "symbol": symbol_summary_json(symbol),
                "kind": details.kind,
                "signature": details.signature,
                "snippet": details.snippet,
                "callers": callers,
                "dependencies": dependencies,
            })
        })
        .collect::<Vec<_>>();

    tool_result(json!({
        "symbol_name": args.symbol_name,
        "matches": payload_matches,
        "warning": warning,
    }))
}

fn handle_get_file_ast(arguments: Value, index: &WorkspaceIndex) -> Result<Value, JsonRpcError> {
    let args: GetFileAstArguments = serde_json::from_value(arguments)
        .map_err(|err| JsonRpcError::invalid_params(format!("invalid get_file_ast args: {err}")))?;
    let path = resolve_workspace_file(index.root(), &args.file_path)?;
    let relative = relative_workspace_path(index.root(), &path);
    let content = fs::read_to_string(&path)
        .map_err(|err| JsonRpcError::invalid_params(format!("unable to read {relative}: {err}")))?;
    let parsed = parse_source_summary(&relative, &content);

    let symbols = parsed
        .symbols
        .iter()
        .map(|symbol| {
            json!({
                "name": symbol.name,
                "kind": symbol.kind,
                "visibility": symbol.visibility,
                "line": symbol.line,
                "signature": symbol.signature,
                "body": args.include_bodies.then(|| extract_symbol_body(&content, symbol.line)),
            })
        })
        .collect::<Vec<_>>();

    tool_result(json!({
        "file_path": parsed.path,
        "language": parsed.language,
        "imports": parsed.imports,
        "symbols": symbols,
    }))
}

fn handle_find_similar_patterns(
    arguments: Value,
    index: &WorkspaceIndex,
) -> Result<Value, JsonRpcError> {
    let args: FindSimilarPatternsArguments = serde_json::from_value(arguments).map_err(|err| {
        JsonRpcError::invalid_params(format!("invalid find_similar_patterns args: {err}"))
    })?;
    let mut results = index.semantic_search(&args.reference, args.max_results.saturating_mul(4));
    results.retain(|result| result.score >= args.min_similarity);
    results.truncate(args.max_results as usize);

    tool_result(json!({
        "reference": args.reference,
        "results": results
            .iter()
            .map(|result| {
                let details = lookup_symbol_details(index.root(), &result.symbol);
                json!({
                    "symbol": result.symbol.id.symbol_name,
                    "kind": details.kind,
                    "file": result.symbol.id.file_path,
                    "line": result.symbol.line,
                    "score": result.score,
                    "snippet": details.snippet,
                })
            })
            .collect::<Vec<_>>(),
    }))
}

fn handle_get_index_stats(index: &WorkspaceIndex) -> Result<Value, JsonRpcError> {
    tool_result(index.stats())
}

fn handle_find_references(arguments: Value, index: &WorkspaceIndex) -> Result<Value, JsonRpcError> {
    let args: FindReferencesArguments = serde_json::from_value(arguments).map_err(|err| {
        JsonRpcError::invalid_params(format!("invalid find_references args: {err}"))
    })?;
    let results = index
        .references_for_symbol(
            &args.symbol_name,
            args.file_path.as_deref(),
            args.include_definitions,
        )
        .map_err(|err| JsonRpcError::invalid_params(err.to_string()))?;
    tool_result(json!({
        "symbol_name": args.symbol_name,
        "results": results,
    }))
}

fn handle_find_implementations(
    arguments: Value,
    index: &WorkspaceIndex,
) -> Result<Value, JsonRpcError> {
    let args: FindImplementationsArguments = serde_json::from_value(arguments).map_err(|err| {
        JsonRpcError::invalid_params(format!("invalid find_implementations args: {err}"))
    })?;
    let mut results = index.implementations_for_trait(&args.trait_name);
    if !args.include_methods {
        for implementation in &mut results {
            implementation.methods.clear();
        }
    }
    tool_result(json!({
        "trait_name": args.trait_name,
        "results": results,
    }))
}

fn handle_get_callers(arguments: Value, index: &WorkspaceIndex) -> Result<Value, JsonRpcError> {
    let args: GetCallersArguments = serde_json::from_value(arguments)
        .map_err(|err| JsonRpcError::invalid_params(format!("invalid get_callers args: {err}")))?;
    let depth = if args.transitive { args.max_depth } else { 1 };
    let graph = index.call_graph(&args.function_name, depth);
    let filtered_roots = filter_roots_by_file(&graph.roots, args.file_path.as_deref());
    let warning = if let Some(file_path) = args.file_path.as_deref() {
        if !graph.roots.is_empty() && filtered_roots.len() != graph.roots.len() {
            Some(format!(
                "callers are derived from the shared function-name graph; roots were filtered to {file_path}"
            ))
        } else {
            None
        }
    } else {
        None
    };

    tool_result(json!({
        "function_name": args.function_name,
        "max_depth": depth,
        "roots": filtered_roots
            .iter()
            .map(|symbol| symbol_summary_json(symbol))
            .collect::<Vec<_>>(),
        "callers": graph.callers.iter().map(symbol_summary_json).collect::<Vec<_>>(),
        "edges": graph
            .edges
            .iter()
            .filter(|edge| edge.direction == roko_index::CallGraphDirection::Callers)
            .map(call_graph_edge_json)
            .collect::<Vec<_>>(),
        "warning": warning,
    }))
}

fn handle_workspace_map(arguments: Value, index: &WorkspaceIndex) -> Result<Value, JsonRpcError> {
    let args: WorkspaceMapArguments = serde_json::from_value(arguments).map_err(|err| {
        JsonRpcError::invalid_params(format!("invalid workspace_map args: {err}"))
    })?;
    let files = collect_workspace_source_files(index.root()).map_err(|err| {
        JsonRpcError::internal_error(format!(
            "unable to scan workspace {}: {err}",
            index.root().display()
        ))
    })?;

    let mut groups: BTreeMap<String, Vec<ParsedFile>> = BTreeMap::new();
    for file in files {
        let relative = relative_workspace_path(index.root(), &file);
        if let Some(focus) = args.focus_path.as_deref() {
            if !file_matches_filter(&relative, focus) && !relative.starts_with(focus) {
                continue;
            }
        }

        let content = match fs::read_to_string(&file) {
            Ok(content) => content,
            Err(err) => {
                tracing::debug!(error = %err, path = %file.display(), "skip unreadable workspace file");
                continue;
            }
        };
        let parsed = parse_source_summary(&relative, &content);
        let key = match args.depth {
            WorkspaceMapDepth::Crate => crate_group_key(&relative),
            WorkspaceMapDepth::Module => module_group_key(&relative),
            WorkspaceMapDepth::Symbol => parsed.path.clone(),
        };
        groups.entry(key).or_default().push(parsed);
    }

    let nodes = groups
        .into_iter()
        .map(|(path, parsed_files)| {
            let file_count = parsed_files.len();
            let symbol_count = parsed_files
                .iter()
                .map(|parsed| parsed.symbols.len())
                .sum::<usize>();
            let symbols = parsed_files
                .iter()
                .flat_map(|parsed| parsed.symbols.iter())
                .take(12)
                .map(|symbol| {
                    json!({
                        "name": symbol.name,
                        "kind": symbol.kind,
                        "line": symbol.line,
                    })
                })
                .collect::<Vec<_>>();
            json!({
                "path": path,
                "file_count": file_count,
                "symbol_count": symbol_count,
                "symbols": symbols,
            })
        })
        .collect::<Vec<_>>();

    tool_result(json!({
        "depth": args.depth,
        "focus_path": args.focus_path,
        "nodes": nodes,
    }))
}

fn handle_get_context(arguments: Value, index: &WorkspaceIndex) -> Result<Value, JsonRpcError> {
    let args: GetContextArguments = serde_json::from_value(arguments)
        .map_err(|err| JsonRpcError::invalid_params(format!("invalid get_context args: {err}")))?;
    let search_args = SearchCodeArguments {
        query: args.task.clone(),
        strategy: SearchStrategy::Hybrid,
        max_results: 8,
        file_pattern: None,
        kind_filter: None,
    };
    let mut hits = search_keyword_hits(index, &search_args)?;
    hits.extend(search_semantic_hits(index, &search_args));
    dedupe_and_sort_hits(&mut hits, search_args.max_results);

    if !args.include_tests {
        hits.retain(|hit| !is_test_path(&hit.file));
    }

    let char_budget = args.token_budget.saturating_mul(4).max(1);
    let mut used_chars = 0_usize;
    let mut included = Vec::new();
    let mut blocks = Vec::new();

    for hit in hits {
        let block = format!(
            "### {} ({})\n{}:{}\n{}\n",
            hit.symbol, hit.kind, hit.file, hit.line, hit.snippet
        );
        if !blocks.is_empty() && used_chars.saturating_add(block.len()) > char_budget {
            break;
        }
        used_chars = used_chars.saturating_add(block.len());
        included.push(json!({
            "symbol": hit.symbol,
            "kind": hit.kind,
            "file": hit.file,
            "line": hit.line,
            "score": hit.score,
        }));
        blocks.push(block);
    }

    let context = format!("Task: {}\n\n{}", args.task, blocks.join("\n"));
    tool_result(json!({
        "task": args.task,
        "token_budget": args.token_budget,
        "used_chars": used_chars,
        "results": included,
        "context": context,
    }))
}

fn handle_symbol_lookup(arguments: Value, index: &WorkspaceIndex) -> Result<Value, JsonRpcError> {
    let args: SymbolLookupArguments = serde_json::from_value(arguments).map_err(|err| {
        JsonRpcError::invalid_params(format!("invalid symbol_lookup args: {err}"))
    })?;
    let results = index.symbol_lookup(&args.name);
    tool_result(json!({
        "query": args.name,
        "results": results,
    }))
}

fn handle_call_graph(arguments: Value, index: &WorkspaceIndex) -> Result<Value, JsonRpcError> {
    let args: CallGraphArguments = serde_json::from_value(arguments)
        .map_err(|err| JsonRpcError::invalid_params(format!("invalid call_graph args: {err}")))?;
    let graph = index.call_graph(&args.function, args.depth);
    tool_result(graph)
}

fn handle_imports(arguments: Value, index: &WorkspaceIndex) -> Result<Value, JsonRpcError> {
    let args: ImportsArguments = serde_json::from_value(arguments)
        .map_err(|err| JsonRpcError::invalid_params(format!("invalid imports args: {err}")))?;
    let imports = index
        .imports_for_file(&args.file)
        .map_err(|err| JsonRpcError::invalid_params(err.to_string()))?;
    tool_result(json!({
        "file": args.file,
        "imports": imports,
    }))
}

fn handle_semantic_search(arguments: Value, index: &WorkspaceIndex) -> Result<Value, JsonRpcError> {
    let args: SemanticSearchArguments = serde_json::from_value(arguments).map_err(|err| {
        JsonRpcError::invalid_params(format!("invalid semantic_search args: {err}"))
    })?;
    let results = index.semantic_search(&args.query, args.limit);
    tool_result(json!({
        "query": args.query,
        "results": results,
    }))
}

fn search_keyword_hits(
    index: &WorkspaceIndex,
    args: &SearchCodeArguments,
) -> Result<Vec<SearchHit>, JsonRpcError> {
    let files = collect_workspace_source_files(index.root()).map_err(|err| {
        JsonRpcError::internal_error(format!(
            "unable to scan workspace {}: {err}",
            index.root().display()
        ))
    })?;
    let query = args.query.to_ascii_lowercase();
    let mut hits = Vec::new();

    for file in files {
        let relative = relative_workspace_path(index.root(), &file);
        if !matches_optional_file_pattern(args.file_pattern.as_deref(), &relative) {
            continue;
        }

        let content = match fs::read_to_string(&file) {
            Ok(content) => content,
            Err(err) => {
                tracing::debug!(error = %err, path = %file.display(), "skip unreadable workspace file");
                continue;
            }
        };
        let parsed = parse_source_summary(&relative, &content);

        for symbol in parsed.symbols {
            if !symbol_kind_matches(args.kind_filter.as_deref(), &symbol.kind) {
                continue;
            }
            let name = symbol.name.to_ascii_lowercase();
            let signature = symbol.signature.to_ascii_lowercase();
            if name == query {
                hits.push(SearchHit {
                    symbol: symbol.name,
                    kind: symbol.kind,
                    file: relative.clone(),
                    line: symbol.line,
                    score: 1.0,
                    snippet: context_snippet(&content, symbol.line, 2),
                });
            } else if name.contains(&query) {
                hits.push(SearchHit {
                    symbol: symbol.name,
                    kind: symbol.kind,
                    file: relative.clone(),
                    line: symbol.line,
                    score: 0.9,
                    snippet: context_snippet(&content, symbol.line, 2),
                });
            } else if signature.contains(&query) {
                hits.push(SearchHit {
                    symbol: symbol.name,
                    kind: symbol.kind,
                    file: relative.clone(),
                    line: symbol.line,
                    score: 0.75,
                    snippet: context_snippet(&content, symbol.line, 2),
                });
            }
        }
    }

    Ok(hits)
}

fn search_semantic_hits(index: &WorkspaceIndex, args: &SearchCodeArguments) -> Vec<SearchHit> {
    let limit = args.max_results.max(1).saturating_mul(4);
    let mut hits = Vec::new();
    for result in index.semantic_search(&args.query, limit) {
        let details = lookup_symbol_details(index.root(), &result.symbol);
        if !matches_optional_file_pattern(args.file_pattern.as_deref(), &result.symbol.id.file_path)
        {
            continue;
        }
        if !symbol_kind_matches(args.kind_filter.as_deref(), &details.kind) {
            continue;
        }
        hits.push(SearchHit {
            symbol: result.symbol.id.symbol_name.clone(),
            kind: details.kind,
            file: result.symbol.id.file_path.clone(),
            line: result.symbol.line,
            score: result.score,
            snippet: details.snippet,
        });
    }
    hits
}

fn dedupe_and_sort_hits(hits: &mut Vec<SearchHit>, max_results: u32) -> usize {
    let mut deduped: BTreeMap<(String, usize, String), SearchHit> = BTreeMap::new();
    for hit in hits.drain(..) {
        let key = (hit.file.clone(), hit.line, hit.symbol.clone());
        match deduped.get_mut(&key) {
            Some(existing) if existing.score < hit.score => *existing = hit,
            Some(_) => {}
            None => {
                deduped.insert(key, hit);
            }
        }
    }

    let total_candidates = deduped.len();
    let mut values = deduped.into_values().collect::<Vec<_>>();
    values.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.file.cmp(&right.file))
            .then_with(|| left.line.cmp(&right.line))
            .then_with(|| left.symbol.cmp(&right.symbol))
    });
    values.truncate(max_results as usize);
    *hits = values;
    total_candidates
}

fn search_hit_json(hit: &SearchHit) -> Value {
    json!({
        "symbol": hit.symbol,
        "kind": hit.kind,
        "file": hit.file,
        "line": hit.line,
        "score": hit.score,
        "snippet": hit.snippet,
    })
}

fn symbol_summary_json(symbol: &roko_index::SymbolInfo) -> Value {
    json!({
        "name": symbol.id.symbol_name,
        "file": symbol.id.file_path,
        "line": symbol.line,
        "visibility": symbol.visibility,
        "language": symbol.language,
    })
}

fn call_graph_edge_json(edge: &roko_index::CallGraphEdge) -> Value {
    json!({
        "from": symbol_summary_json(&edge.from),
        "to": symbol_summary_json(&edge.to),
        "direction": edge.direction,
        "depth": edge.depth,
    })
}

fn filter_roots_by_file<'a>(
    roots: &'a [roko_index::SymbolInfo],
    file_path: Option<&str>,
) -> Vec<&'a roko_index::SymbolInfo> {
    file_path.map_or_else(
        || roots.iter().collect(),
        |file_path| {
            roots
                .iter()
                .filter(|root| file_matches_filter(&root.id.file_path, file_path))
                .collect()
        },
    )
}

fn lookup_symbol_details(root: &Path, symbol: &roko_index::SymbolInfo) -> ParsedSymbolDetails {
    let path = root.join(&symbol.id.file_path);
    let Ok(content) = fs::read_to_string(path) else {
        return ParsedSymbolDetails {
            kind: "unknown".to_string(),
            signature: symbol.id.symbol_name.clone(),
            snippet: String::new(),
        };
    };
    let parsed = parse_source_summary(&symbol.id.file_path, &content);
    let matched = parsed
        .symbols
        .iter()
        .find(|candidate| candidate.line == symbol.line && candidate.name == symbol.id.symbol_name)
        .or_else(|| {
            parsed
                .symbols
                .iter()
                .find(|candidate| candidate.name == symbol.id.symbol_name)
        });

    matched.map_or_else(
        || ParsedSymbolDetails {
            kind: "unknown".to_string(),
            signature: symbol.id.symbol_name.clone(),
            snippet: context_snippet(&content, symbol.line, 2),
        },
        |parsed_symbol| ParsedSymbolDetails {
            kind: parsed_symbol.kind.clone(),
            signature: parsed_symbol.signature.clone(),
            snippet: context_snippet(&content, parsed_symbol.line, 2),
        },
    )
}

struct ParsedSymbolDetails {
    kind: String,
    signature: String,
    snippet: String,
}

fn collect_workspace_source_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    collect_workspace_source_files_in_dir(root, root, &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_workspace_source_files_in_dir(
    root: &Path,
    dir: &Path,
    out: &mut Vec<PathBuf>,
) -> Result<()> {
    for entry in fs::read_dir(dir)? {
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
            collect_workspace_source_files_in_dir(root, &path, out)?;
            continue;
        }

        if file_type.is_file() && is_supported_source_file(&path) {
            let relative = path.strip_prefix(root).unwrap_or(&path);
            out.push(root.join(relative));
        }
    }

    Ok(())
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

fn is_supported_source_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some("rs" | "ts" | "tsx" | "js" | "jsx" | "go")
    )
}

fn resolve_workspace_file(root: &Path, file: &str) -> Result<PathBuf, JsonRpcError> {
    let candidate = Path::new(file);
    let absolute = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        root.join(candidate)
    };
    let canonical = fs::canonicalize(&absolute).map_err(|err| {
        JsonRpcError::invalid_params(format!("resolve workspace file {file}: {err}"))
    })?;
    if !canonical.starts_with(root) {
        return Err(JsonRpcError::invalid_params(format!(
            "file '{file}' is outside workspace"
        )));
    }
    Ok(canonical)
}

fn relative_workspace_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn parse_source_summary(path: &str, content: &str) -> ParsedFile {
    match path_extension(path) {
        Some("rs") => parse_rust_summary(path, content),
        Some("ts" | "tsx" | "js" | "jsx") => parse_typescript_summary(path, content),
        Some("go") => parse_go_summary(path, content),
        _ => ParsedFile {
            path: path.to_string(),
            language: "unknown".to_string(),
            imports: Vec::new(),
            symbols: Vec::new(),
        },
    }
}

fn path_extension(path: &str) -> Option<&str> {
    Path::new(path).extension().and_then(|ext| ext.to_str())
}

#[allow(clippy::too_many_lines)]
fn parse_rust_summary(path: &str, content: &str) -> ParsedFile {
    let mut imports = Vec::new();
    let mut symbols = Vec::new();

    for (line_no, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("use ") {
            imports.push(rest.trim_end_matches(';').trim().to_string());
        } else if let Some(rest) = trimmed.strip_prefix("extern crate ") {
            imports.push(rest.trim_end_matches(';').trim().to_string());
        } else if let Some(rest) = trimmed.strip_prefix("pub use ") {
            imports.push(rest.trim_end_matches(';').trim().to_string());
        }

        let (visibility, rest) = rust_visibility(trimmed);
        if let Some(name) = rest
            .strip_prefix("async fn ")
            .or_else(|| rest.strip_prefix("fn "))
            .and_then(parse_identifier)
        {
            symbols.push(parsed_symbol(
                name,
                "function",
                visibility,
                line_no + 1,
                trimmed,
            ));
            continue;
        }
        if let Some(name) = rest.strip_prefix("struct ").and_then(parse_identifier) {
            symbols.push(parsed_symbol(
                name,
                "struct",
                visibility,
                line_no + 1,
                trimmed,
            ));
            continue;
        }
        if let Some(name) = rest.strip_prefix("enum ").and_then(parse_identifier) {
            symbols.push(parsed_symbol(
                name,
                "enum",
                visibility,
                line_no + 1,
                trimmed,
            ));
            continue;
        }
        if let Some(name) = rest.strip_prefix("trait ").and_then(parse_identifier) {
            symbols.push(parsed_symbol(
                name,
                "trait",
                visibility,
                line_no + 1,
                trimmed,
            ));
            continue;
        }
        if let Some(name) = rest.strip_prefix("const ").and_then(parse_identifier) {
            symbols.push(parsed_symbol(
                name,
                "const",
                visibility,
                line_no + 1,
                trimmed,
            ));
            continue;
        }
        if let Some(name) = rest.strip_prefix("type ").and_then(parse_identifier) {
            symbols.push(parsed_symbol(
                name,
                "type",
                visibility,
                line_no + 1,
                trimmed,
            ));
            continue;
        }
        if let Some(name) = rest.strip_prefix("mod ").and_then(parse_identifier) {
            symbols.push(parsed_symbol(
                name,
                "module",
                visibility,
                line_no + 1,
                trimmed,
            ));
            continue;
        }
        if let Some(name) = rest.strip_prefix("impl ").map(parse_impl_target) {
            symbols.push(parsed_symbol(
                name,
                "impl",
                visibility,
                line_no + 1,
                trimmed,
            ));
        }
    }

    ParsedFile {
        path: path.to_string(),
        language: "rust".to_string(),
        imports,
        symbols,
    }
}

fn parse_typescript_summary(path: &str, content: &str) -> ParsedFile {
    let mut imports = Vec::new();
    let mut symbols = Vec::new();

    for (line_no, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("import ") || trimmed.starts_with("export * from ") {
            imports.push(trimmed.trim_end_matches(';').to_string());
        }

        let rest = trimmed
            .strip_prefix("export default ")
            .or_else(|| trimmed.strip_prefix("export "))
            .unwrap_or(trimmed);
        if let Some(name) = rest
            .strip_prefix("async function ")
            .or_else(|| rest.strip_prefix("function "))
            .and_then(parse_identifier)
        {
            symbols.push(parsed_symbol(
                name,
                "function",
                "public",
                line_no + 1,
                trimmed,
            ));
            continue;
        }
        if let Some(name) = rest.strip_prefix("class ").and_then(parse_identifier) {
            symbols.push(parsed_symbol(
                name,
                "struct",
                "public",
                line_no + 1,
                trimmed,
            ));
            continue;
        }
        if let Some(name) = rest.strip_prefix("interface ").and_then(parse_identifier) {
            symbols.push(parsed_symbol(name, "trait", "public", line_no + 1, trimmed));
            continue;
        }
        if let Some(name) = rest.strip_prefix("enum ").and_then(parse_identifier) {
            symbols.push(parsed_symbol(name, "enum", "public", line_no + 1, trimmed));
            continue;
        }
        if let Some(name) = rest.strip_prefix("type ").and_then(parse_identifier) {
            symbols.push(parsed_symbol(name, "type", "public", line_no + 1, trimmed));
            continue;
        }
        if let Some(name) = rest.strip_prefix("const ").and_then(parse_identifier) {
            symbols.push(parsed_symbol(name, "const", "public", line_no + 1, trimmed));
        }
    }

    ParsedFile {
        path: path.to_string(),
        language: "typescript".to_string(),
        imports,
        symbols,
    }
}

fn parse_go_summary(path: &str, content: &str) -> ParsedFile {
    let mut imports = Vec::new();
    let mut symbols = Vec::new();
    let mut in_import_block = false;

    for (line_no, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed == "import (" {
            in_import_block = true;
            continue;
        }
        if in_import_block {
            if trimmed == ")" {
                in_import_block = false;
            } else if let Some(path) = parse_go_import_path(trimmed) {
                imports.push(path.to_string());
            }
            continue;
        }
        if let Some(path) = trimmed
            .strip_prefix("import ")
            .and_then(parse_go_import_path)
        {
            imports.push(path.to_string());
        }

        if let Some(rest) = trimmed.strip_prefix("func ") {
            let rest = if rest.starts_with('(') {
                rest.split_once(')')
                    .map_or(rest, |(_, tail)| tail.trim_start())
            } else {
                rest
            };
            if let Some(name) = parse_identifier(rest) {
                symbols.push(parsed_symbol(
                    name,
                    "function",
                    "public",
                    line_no + 1,
                    trimmed,
                ));
                continue;
            }
        }
        if let Some(rest) = trimmed.strip_prefix("type ") {
            if let Some(name) = parse_identifier(rest) {
                let after_name = rest.trim_start_matches(name).trim_start();
                let kind = if after_name.starts_with("struct") {
                    "struct"
                } else if after_name.starts_with("interface") {
                    "trait"
                } else {
                    "type"
                };
                symbols.push(parsed_symbol(name, kind, "public", line_no + 1, trimmed));
                continue;
            }
        }
        if let Some(name) = trimmed.strip_prefix("const ").and_then(parse_identifier) {
            symbols.push(parsed_symbol(name, "const", "public", line_no + 1, trimmed));
        }
    }

    ParsedFile {
        path: path.to_string(),
        language: "go".to_string(),
        imports,
        symbols,
    }
}

fn rust_visibility(trimmed: &str) -> (&str, &str) {
    trimmed
        .strip_prefix("pub(crate) ")
        .or_else(|| trimmed.strip_prefix("pub(super) "))
        .or_else(|| trimmed.strip_prefix("pub "))
        .map_or(("private", trimmed), |rest| ("public", rest))
}

fn parse_identifier(rest: &str) -> Option<&str> {
    let end = rest
        .char_indices()
        .find(|(_, ch)| !(ch.is_ascii_alphanumeric() || *ch == '_'))
        .map_or(rest.len(), |(idx, _)| idx);
    if end == 0 { None } else { Some(&rest[..end]) }
}

fn parse_impl_target(rest: &str) -> &str {
    let trimmed = rest.trim_start();
    if let Some((_, target)) = trimmed.split_once(" for ") {
        parse_identifier(target.trim_start()).unwrap_or(trimmed)
    } else {
        parse_identifier(trimmed).unwrap_or(trimmed)
    }
}

fn parse_go_import_path(rest: &str) -> Option<&str> {
    let first_quote = rest.find('"')?;
    let tail = &rest[first_quote + 1..];
    let last_quote = tail.find('"')?;
    Some(&tail[..last_quote])
}

fn parsed_symbol(
    name: &str,
    kind: &str,
    visibility: &str,
    line: usize,
    signature: &str,
) -> ParsedSymbol {
    ParsedSymbol {
        name: name.to_string(),
        kind: kind.to_string(),
        visibility: visibility.to_string(),
        line,
        signature: signature.to_string(),
    }
}

fn extract_symbol_body(content: &str, line: usize) -> String {
    let lines = content.lines().collect::<Vec<_>>();
    if line == 0 || line > lines.len() {
        return String::new();
    }

    let mut body = Vec::new();
    let mut balance = 0_i32;
    let mut saw_brace = false;
    for line_text in lines.into_iter().skip(line - 1).take(32) {
        body.push(line_text);
        for ch in line_text.chars() {
            if ch == '{' {
                saw_brace = true;
                balance += 1;
            } else if ch == '}' {
                balance -= 1;
            }
        }
        if saw_brace && balance <= 0 {
            break;
        }
        if !saw_brace && line_text.trim_end().ends_with(';') {
            break;
        }
    }

    body.join("\n")
}

fn context_snippet(content: &str, line: usize, context_lines: usize) -> String {
    let lines = content.lines().collect::<Vec<_>>();
    if lines.is_empty() {
        return String::new();
    }
    let start = line.saturating_sub(context_lines + 1);
    let end = usize::min(lines.len(), line.saturating_add(context_lines));
    lines[start..end].join("\n")
}

fn matches_optional_file_pattern(pattern: Option<&str>, path: &str) -> bool {
    pattern.is_none_or(|pattern| wildcard_match(pattern, path))
}

fn file_matches_filter(path: &str, filter: &str) -> bool {
    wildcard_match(filter, path)
        || normalize_path(path) == normalize_path(filter)
        || normalize_path(path).ends_with(&normalize_path(filter))
}

fn wildcard_match(pattern: &str, value: &str) -> bool {
    wildcard_match_bytes(pattern.as_bytes(), value.as_bytes())
}

fn wildcard_match_bytes(pattern: &[u8], value: &[u8]) -> bool {
    let (mut pattern_index, mut value_index, mut wildcard_pattern_index, mut wildcard_value_index) =
        (0_usize, 0_usize, None, 0_usize);
    while value_index < value.len() {
        if pattern_index < pattern.len()
            && (pattern[pattern_index] == b'?' || pattern[pattern_index] == value[value_index])
        {
            pattern_index += 1;
            value_index += 1;
        } else if pattern_index < pattern.len() && pattern[pattern_index] == b'*' {
            wildcard_pattern_index = Some(pattern_index);
            pattern_index += 1;
            wildcard_value_index = value_index;
        } else if let Some(wildcard_index) = wildcard_pattern_index {
            pattern_index = wildcard_index + 1;
            wildcard_value_index += 1;
            value_index = wildcard_value_index;
        } else {
            return false;
        }
    }

    while pattern_index < pattern.len() && pattern[pattern_index] == b'*' {
        pattern_index += 1;
    }
    pattern_index == pattern.len()
}

fn symbol_kind_matches(filter: Option<&str>, kind: &str) -> bool {
    filter.is_none_or(|filter| filter.eq_ignore_ascii_case(kind))
}

fn crate_group_key(path: &str) -> String {
    let mut parts = path.split('/');
    match (parts.next(), parts.next()) {
        (Some("crates"), Some(name)) => format!("crates/{name}"),
        (Some("apps"), Some(name)) => format!("apps/{name}"),
        (Some(first), Some(second)) => format!("{first}/{second}"),
        (Some(first), None) => first.to_string(),
        _ => ".".to_string(),
    }
}

fn module_group_key(path: &str) -> String {
    Path::new(path)
        .parent()
        .map(|parent| parent.to_string_lossy().replace('\\', "/"))
        .filter(|parent| !parent.is_empty())
        .unwrap_or_else(|| ".".to_string())
}

fn is_test_path(path: &str) -> bool {
    path.contains("/tests/")
        || path.ends_with("_test.rs")
        || path.ends_with(".test.ts")
        || path.ends_with(".spec.ts")
        || path.ends_with("_test.go")
}

fn normalize_path(path: &str) -> String {
    path.replace('\\', "/").trim_start_matches("./").to_string()
}

fn tool_spec(name: &str, description: &str, input_schema: &Value) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": input_schema,
    })
}

fn tool_result<T: Serialize>(payload: T) -> Result<Value, JsonRpcError> {
    let text = serde_json::to_string(&payload)
        .map_err(|err| JsonRpcError::internal_error(err.to_string()))?;
    Ok(json!({
        "content": [{
            "type": "text",
            "text": text
        }],
        "isError": false
    }))
}

fn empty_json_object() -> Value {
    Value::Object(serde_json::Map::default())
}

const fn default_search_strategy() -> SearchStrategy {
    SearchStrategy::Hybrid
}

const fn default_max_results() -> u32 {
    10
}

const fn default_true() -> bool {
    true
}

const fn default_expansion_depth() -> u32 {
    1
}

const fn default_min_similarity() -> f64 {
    0.6
}

const fn default_max_depth() -> u32 {
    2
}

const fn default_workspace_map_depth() -> WorkspaceMapDepth {
    WorkspaceMapDepth::Module
}

const fn default_token_budget() -> usize {
    40_000
}

const fn effective_search_strategy(strategy: SearchStrategy) -> SearchStrategy {
    match strategy {
        SearchStrategy::Keyword => SearchStrategy::Keyword,
        SearchStrategy::Structural => SearchStrategy::Structural,
        SearchStrategy::Hdc | SearchStrategy::Embedding => SearchStrategy::Hdc,
        SearchStrategy::Hybrid => SearchStrategy::Hybrid,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::language::{Symbol, SymbolKind, Visibility};
    use serde_json::json;
    use std::fs;
    use std::io::Cursor;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn make_index() -> WorkspaceIndex {
        WorkspaceIndex::from_source_files(vec![roko_index::SourceFile {
            path: "src/lib.rs".to_string(),
            language: "rust".to_string(),
            content: "fn helper() {}\nfn main() { helper(); }\n".to_string(),
            symbols: vec![
                Symbol {
                    name: "helper".to_string(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    line: 1,
                },
                Symbol {
                    name: "main".to_string(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    line: 2,
                },
            ],
            imports: Vec::new(),
        }])
    }

    fn make_workspace(files: &[(&str, &str)]) -> (PathBuf, WorkspaceIndex) {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("roko_mcp_code_{unique}"));
        fs::create_dir_all(&root).expect("create temp root");
        for (path, content) in files {
            let full_path = root.join(path);
            if let Some(parent) = full_path.parent() {
                fs::create_dir_all(parent).expect("create parent dirs");
            }
            fs::write(&full_path, content).expect("write temp file");
        }
        let index = WorkspaceIndex::load(&root).expect("load workspace index");
        (root, index)
    }

    #[test]
    fn tools_list_includes_documented_tools() {
        let tools = handle_tools_list();
        let names = tools["tools"]
            .as_array()
            .expect("tools array")
            .iter()
            .map(|tool| tool["name"].as_str().expect("tool name"))
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            vec![
                "search_code",
                "get_symbol_context",
                "get_file_ast",
                "find_similar_patterns",
                "get_callers",
                "workspace_map",
                "get_context",
            ]
        );
    }

    #[test]
    fn search_code_tool_returns_matches() {
        let index = make_index();
        let result = dispatch_tool_call(
            "search_code",
            json!({ "query": "helper", "strategy": "hybrid", "max_results": 5 }),
            &index,
        )
        .expect("tool result");

        let payload: Value =
            serde_json::from_str(result["content"][0]["text"].as_str().expect("payload text"))
                .expect("json payload");
        assert_eq!(payload["strategy_used"], "hybrid");
        assert_eq!(payload["results"].as_array().expect("results").len(), 1);
    }

    #[test]
    fn get_symbol_context_returns_matches() {
        let index = make_index();
        let result = dispatch_tool_call(
            "get_symbol_context",
            json!({ "symbol_name": "helper" }),
            &index,
        )
        .expect("tool result");

        let payload: Value =
            serde_json::from_str(result["content"][0]["text"].as_str().expect("payload text"))
                .expect("json payload");
        assert_eq!(payload["symbol_name"], "helper");
        assert_eq!(payload["matches"].as_array().expect("matches").len(), 1);
    }

    #[test]
    fn get_file_ast_reads_workspace_file() {
        let (_root, index) = make_workspace(&[(
            "src/lib.rs",
            "use crate::helper::Helper;\npub fn helper() {}\nstruct Local;\n",
        )]);
        let result = dispatch_tool_call(
            "get_file_ast",
            json!({ "file_path": "src/lib.rs", "include_bodies": false }),
            &index,
        )
        .expect("tool result");

        let payload: Value =
            serde_json::from_str(result["content"][0]["text"].as_str().expect("payload text"))
                .expect("json payload");
        assert_eq!(payload["language"], "rust");
        assert_eq!(payload["symbols"].as_array().expect("symbols").len(), 2);
    }

    #[test]
    fn get_callers_returns_callers_only() {
        let index = make_index();
        let result = dispatch_tool_call(
            "get_callers",
            json!({ "function_name": "helper", "transitive": false }),
            &index,
        )
        .expect("tool result");
        let payload: Value =
            serde_json::from_str(result["content"][0]["text"].as_str().expect("payload text"))
                .expect("json payload");
        assert_eq!(payload["function_name"], "helper");
        assert_eq!(payload["callers"].as_array().expect("callers").len(), 1);
    }

    #[test]
    fn workspace_map_groups_workspace_files() {
        let (_root, index) = make_workspace(&[
            ("crates/demo/src/lib.rs", "pub fn helper() {}\n"),
            ("crates/demo/src/extra.rs", "pub struct Demo {}\n"),
        ]);
        let result = dispatch_tool_call("workspace_map", json!({ "depth": "crate" }), &index)
            .expect("tool result");

        let payload: Value =
            serde_json::from_str(result["content"][0]["text"].as_str().expect("payload text"))
                .expect("json payload");
        assert_eq!(payload["depth"], "crate");
        assert_eq!(payload["nodes"].as_array().expect("nodes").len(), 1);
    }

    #[test]
    fn get_context_respects_budget() {
        let (_root, index) = make_workspace(&[(
            "src/lib.rs",
            "pub fn helper() {\n    println!(\"hi\");\n}\n\npub fn main_task() {\n    helper();\n}\n",
        )]);
        let result = dispatch_tool_call(
            "get_context",
            json!({ "task": "helper", "token_budget": 20 }),
            &index,
        )
        .expect("tool result");

        let payload: Value =
            serde_json::from_str(result["content"][0]["text"].as_str().expect("payload text"))
                .expect("json payload");
        assert!(payload["used_chars"].as_u64().expect("used chars") <= 80);
    }

    #[test]
    fn symbol_lookup_tool_returns_matches() {
        let index = make_index();
        let result = dispatch_tool_call("symbol_lookup", json!({ "name": "helper" }), &index)
            .expect("tool result");

        let payload: Value =
            serde_json::from_str(result["content"][0]["text"].as_str().expect("payload text"))
                .expect("json payload");
        assert_eq!(payload["query"], "helper");
        assert_eq!(payload["results"].as_array().expect("results").len(), 1);
    }

    #[test]
    fn call_graph_tool_returns_callers_and_callees() {
        let index = make_index();
        let result = dispatch_tool_call(
            "call_graph",
            json!({ "function": "main", "depth": 2 }),
            &index,
        )
        .expect("tool result");
        let payload: Value =
            serde_json::from_str(result["content"][0]["text"].as_str().expect("payload text"))
                .expect("json payload");
        assert_eq!(payload["function"], "main");
        assert_eq!(payload["callees"].as_array().expect("callees").len(), 1);
    }

    #[test]
    fn tools_call_missing_arguments_field_returns_error() {
        let index = make_index();
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "tools/call".to_string(),
            params: json!({ "arguments": {} }),
            id: json!(1),
        };

        let err = match handle_request(request, &index) {
            Ok(value) => panic!(
                "expected invalid tools/call params error for missing arguments field, got success: {value}"
            ),
            Err(err) => err,
        };

        assert_eq!(
            err.code,
            JsonRpcError::INVALID_REQUEST,
            "missing arguments field should be reported as an invalid request"
        );
        assert!(
            err.message.contains("invalid tools/call params"),
            "error message should identify the tools/call parameter shape problem: {}",
            err.message
        );
        assert!(
            err.message.contains("name"),
            "error message should mention the missing name field: {}",
            err.message
        );
    }

    #[test]
    fn search_code_missing_required_arg_returns_error() {
        let index = make_index();
        let err = match dispatch_tool_call("search_code", json!({}), &index) {
            Ok(value) => panic!(
                "expected invalid search_code args error for missing query field, got success: {value}"
            ),
            Err(err) => err,
        };

        assert_eq!(
            err.code,
            JsonRpcError::INVALID_REQUEST,
            "missing search_code.query should be reported as an invalid request"
        );
        assert!(
            err.message.contains("invalid search_code args"),
            "error message should identify the search_code argument parsing problem: {}",
            err.message
        );
        assert!(
            err.message.contains("query"),
            "error message should mention the missing query field: {}",
            err.message
        );
    }

    #[test]
    fn imports_missing_file_returns_error() {
        let index = make_index();
        let err = match dispatch_tool_call("imports", json!({ "file": "missing.rs" }), &index) {
            Ok(value) => panic!(
                "expected invalid imports args error for nonexistent file, got success: {value}"
            ),
            Err(err) => err,
        };

        assert_eq!(
            err.code,
            JsonRpcError::INVALID_REQUEST,
            "nonexistent imports file should be reported as an invalid request"
        );
        assert!(
            err.message.contains("resolve workspace file missing.rs")
                || err.message.contains("file 'missing.rs' was not indexed"),
            "error message should mention the missing file path: {}",
            err.message
        );
    }

    #[test]
    fn malformed_json_returns_parse_error() {
        let mut output = Vec::new();

        serve_stdio(Cursor::new(b"{not json}\n"), &mut output, |_request| {
            panic!("handler should not be called for malformed JSON input");
        })
        .expect("stdio transport");

        let response: Value = serde_json::from_slice(&output)
            .expect("malformed JSON should still produce a structured response");
        assert_eq!(
            response["jsonrpc"], "2.0",
            "malformed JSON should produce a JSON-RPC 2.0 response"
        );
        assert_eq!(
            response["id"],
            Value::Null,
            "parse errors should use a null request id"
        );
        assert_eq!(
            response["error"]["code"],
            JsonRpcError::PARSE_ERROR,
            "malformed JSON should surface as a parse error"
        );
    }
}
