# MCP Context Server Design

> Exposing code intelligence to agents via Model Context Protocol (MCP) tools — the agent-facing API for code search, symbol navigation, impact analysis, and context retrieval.


> **Implementation**: Built

**Topic**: [Code Intelligence](./INDEX.md)
**Prerequisites**: [06-context-assembly-from-code.md](./06-context-assembly-from-code.md)
**Key sources**: `bardo-backup/tmp/mori-refactor/10-code-intelligence.md`, `bardo-backup/tmp/mori-agents/18-code-intelligence-and-gateway.md`, `bardo-backup/tmp/death/tools/02-code-index.md`

---

## Abstract

Code intelligence is useless unless agents can access it. The Model Context Protocol (MCP) provides the mechanism: a standardized interface through which agents invoke tools. The MCP context server wraps `roko-index` capabilities as MCP tools that agents can call during their Synapse Loop execution.

Rather than requiring agents to understand the `SymbolGraph` API, `PageRank` algorithm, or HDC fingerprint system directly, the MCP server presents ten high-level tools: `search_code`, `get_symbol_context`, `get_file_ast`, `find_similar_patterns`, `get_index_stats`, `find_references`, `find_implementations`, `get_callers`, `workspace_map`, and `get_context`. Each tool accepts a JSON input, queries the index, and returns structured output suitable for LLM consumption.

This document describes the ten tools, their input/output schemas, the server architecture, and the integration with Roko's existing MCP passthrough configuration.

---

## The Ten MCP Tools

### Tool 1: search_code

The primary entry point for code search. Combines multiple search strategies via RRF.

```json
{
    "name": "search_code",
    "description": "Search the codebase for symbols, patterns, or code matching a query.",
    "input_schema": {
        "type": "object",
        "properties": {
            "query": {
                "type": "string",
                "description": "Natural language or code query"
            },
            "strategy": {
                "type": "string",
                "enum": ["keyword", "structural", "hdc", "embedding", "hybrid"],
                "default": "hybrid"
            },
            "max_results": {
                "type": "integer",
                "default": 10
            },
            "file_pattern": {
                "type": "string",
                "description": "Glob pattern to scope search (e.g., 'crates/roko-index/**')"
            },
            "kind_filter": {
                "type": "string",
                "enum": ["function", "struct", "enum", "trait", "const", "type", "module", "impl"]
            }
        },
        "required": ["query"]
    }
}
```

**Output**: Ranked list of matching symbols with file paths, line numbers, relevance scores, and code snippets.

**Example invocation**:
```json
{
    "query": "build dependency graph from source files",
    "strategy": "hybrid",
    "max_results": 5,
    "file_pattern": "crates/roko-index/**"
}
```

**Example output**:
```json
{
    "results": [
        {
            "symbol": "build_graph",
            "kind": "function",
            "file": "crates/roko-index/src/graph.rs",
            "line": 118,
            "score": 0.92,
            "snippet": "pub fn build_graph(files: &[SourceFile]) -> SymbolGraph { ... }",
            "context_lines": 5
        },
        {
            "symbol": "SymbolGraph",
            "kind": "struct",
            "file": "crates/roko-index/src/graph.rs",
            "line": 46,
            "score": 0.87,
            "snippet": "pub struct SymbolGraph { nodes: HashSet<SymbolId>, ... }"
        }
    ],
    "total_candidates": 47,
    "strategy_used": "hybrid",
    "elapsed_ms": 12
}
```

### Tool 2: get_symbol_context

Retrieve full context for a specific symbol, including dependencies, callers, and surrounding code.

```json
{
    "name": "get_symbol_context",
    "description": "Get detailed context for a symbol including definition, dependencies, and callers.",
    "input_schema": {
        "type": "object",
        "properties": {
            "symbol_name": {
                "type": "string"
            },
            "file_path": {
                "type": "string",
                "description": "Optional: disambiguate if multiple symbols share the name"
            },
            "include_dependencies": {
                "type": "boolean",
                "default": true
            },
            "include_callers": {
                "type": "boolean",
                "default": true
            },
            "expansion_depth": {
                "type": "integer",
                "default": 1
            }
        },
        "required": ["symbol_name"]
    }
}
```

**Output**: The symbol's full definition, its direct dependencies (forward edges), its callers (reverse edges), PageRank score, and HDC fingerprint similarity to task context.

### Tool 3: get_file_ast

Return the AST structure of a file — useful for understanding file organization without reading every line.

```json
{
    "name": "get_file_ast",
    "description": "Get the symbol-level structure of a source file.",
    "input_schema": {
        "type": "object",
        "properties": {
            "file_path": {
                "type": "string"
            },
            "include_bodies": {
                "type": "boolean",
                "default": false,
                "description": "Include function bodies (verbose) or just signatures"
            }
        },
        "required": ["file_path"]
    }
}
```

**Output**: Hierarchical list of symbols with their kinds, visibility, line numbers, and optionally bodies. This gives the agent a "table of contents" view of a file.

### Tool 4: find_similar_patterns

Find code structurally similar to a given snippet or symbol.

```json
{
    "name": "find_similar_patterns",
    "description": "Find code patterns structurally similar to a reference symbol or code snippet.",
    "input_schema": {
        "type": "object",
        "properties": {
            "reference": {
                "type": "string",
                "description": "Symbol name or code snippet to find similar patterns for"
            },
            "min_similarity": {
                "type": "number",
                "default": 0.6,
                "description": "Minimum HDC similarity score (0.0-1.0)"
            },
            "max_results": {
                "type": "integer",
                "default": 10
            }
        },
        "required": ["reference"]
    }
}
```

**Output**: Ranked list of similar symbols with similarity scores, file locations, and code snippets.

### Tool 5: get_index_stats

Report the state of the code index — useful for agents to understand index coverage.

```json
{
    "name": "get_index_stats",
    "description": "Get statistics about the code index: file count, symbol count, edge count, etc.",
    "input_schema": {
        "type": "object",
        "properties": {}
    }
}
```

**Output**:
```json
{
    "indexed_files": 342,
    "total_symbols": 4891,
    "total_edges": 12340,
    "edge_breakdown": {
        "imports": 12340,
        "calls": 0,
        "implements": 0,
        "contains": 0
    },
    "languages": {
        "rust": 298,
        "typescript": 32,
        "go": 12
    },
    "top_symbols_by_pagerank": [
        { "name": "Signal", "kind": "struct", "score": 0.042 },
        { "name": "Gate", "kind": "trait", "score": 0.031 }
    ],
    "last_indexed": "2026-04-12T10:30:00Z",
    "index_size_bytes": 15728640
}
```

### Tool 6: find_references

Find all usage sites of a symbol.

```json
{
    "name": "find_references",
    "description": "Find all locations where a symbol is referenced (imported, called, or mentioned).",
    "input_schema": {
        "type": "object",
        "properties": {
            "symbol_name": { "type": "string" },
            "file_path": { "type": "string" },
            "include_definitions": {
                "type": "boolean",
                "default": false
            }
        },
        "required": ["symbol_name"]
    }
}
```

**Output**: List of `SymbolRef` locations with file, line, column, and surrounding context line.

### Tool 7: find_implementations

Find all implementations of a trait or interface.

```json
{
    "name": "find_implementations",
    "description": "Find all types that implement a given trait or interface.",
    "input_schema": {
        "type": "object",
        "properties": {
            "trait_name": { "type": "string" },
            "include_methods": {
                "type": "boolean",
                "default": true
            }
        },
        "required": ["trait_name"]
    }
}
```

**Output**: List of implementing types with their file locations and method summaries.

### Tool 8: get_callers

Find all symbols that call a given function or method.

```json
{
    "name": "get_callers",
    "description": "Find all functions that call a given function.",
    "input_schema": {
        "type": "object",
        "properties": {
            "function_name": { "type": "string" },
            "file_path": { "type": "string" },
            "transitive": {
                "type": "boolean",
                "default": false,
                "description": "Include indirect callers (callers of callers)"
            },
            "max_depth": {
                "type": "integer",
                "default": 2
            }
        },
        "required": ["function_name"]
    }
}
```

**Output**: List of calling symbols with call site locations, ordered by graph distance.

### Tool 9: workspace_map

Generate a high-level map of the workspace structure — the Aider "repository map" concept.

```json
{
    "name": "workspace_map",
    "description": "Get a high-level map of the workspace: crates, modules, key types, and their relationships.",
    "input_schema": {
        "type": "object",
        "properties": {
            "depth": {
                "type": "string",
                "enum": ["crate", "module", "symbol"],
                "default": "module"
            },
            "focus_path": {
                "type": "string",
                "description": "Optional: focus the map on a specific crate or module"
            }
        }
    }
}
```

**Output**: Hierarchical representation of the workspace showing crates → modules → top symbols with their dependency relationships and PageRank scores. This gives agents a "birds-eye view" without requiring them to read individual files.

### Tool 10: get_context

The meta-tool: given a task description, assemble the optimal context block automatically.

```json
{
    "name": "get_context",
    "description": "Given a task description, automatically assemble the optimal code context for that task.",
    "input_schema": {
        "type": "object",
        "properties": {
            "task": {
                "type": "string",
                "description": "Natural language description of the task"
            },
            "token_budget": {
                "type": "integer",
                "default": 40000,
                "description": "Maximum tokens for the context block"
            },
            "include_tests": {
                "type": "boolean",
                "default": false
            }
        },
        "required": ["task"]
    }
}
```

**Output**: A fully assembled context block containing ranked code slices, dependency information, and relevant symbols — ready to be inserted into the agent's prompt.

---

## Server Architecture

### MCP server structure

The MCP context server follows the standard MCP stdio transport pattern:

```rust
// Planned: MCP context server entry point
pub struct CodeIntelligenceServer {
    index: Arc<CodeIndex>,      // Shared code index
    graph: Arc<SymbolGraph>,    // Pre-computed dependency graph
    config: ServerConfig,
}

impl CodeIntelligenceServer {
    pub async fn handle_tool_call(
        &self,
        tool_name: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value> {
        match tool_name {
            "search_code" => self.search_code(params).await,
            "get_symbol_context" => self.get_symbol_context(params).await,
            "get_file_ast" => self.get_file_ast(params).await,
            "find_similar_patterns" => self.find_similar(params).await,
            "get_index_stats" => self.get_stats(params).await,
            "find_references" => self.find_refs(params).await,
            "find_implementations" => self.find_impls(params).await,
            "get_callers" => self.get_callers(params).await,
            "workspace_map" => self.workspace_map(params).await,
            "get_context" => self.get_context(params).await,
            _ => Err(Error::UnknownTool(tool_name.into())),
        }
    }
}
```

### Integration with roko.toml MCP config

Roko already supports MCP configuration passthrough via `agent.mcp_config` in `roko.toml`. The code intelligence server would be configured as:

```toml
[agent.mcp_config.servers.code-intelligence]
command = "roko"
args = ["mcp", "code-intelligence"]
env = {}
```

This means the server starts as a child process of the orchestrator and communicates via stdio JSON-RPC. No external infrastructure required.

### Index lifecycle

The server manages the code index lifecycle:

1. **Startup** — Load snapshot from disk (if exists) or build fresh index
2. **Background re-index** — Watch for file changes (via `roko-conductor` watcher) and update incrementally
3. **Query serving** — Handle tool calls from agents concurrently
4. **Shutdown** — Persist index snapshot to disk

```
                Startup
                ───────
  Snapshot exists? ──Yes──→ Load rkyv snapshot ──→ Ready
       │
       No
       │
       ▼
  Full index build ──→ Persist snapshot ──→ Ready

                Runtime
                ───────
  File change detected ──→ Re-parse changed files
                          ──→ Update graph incrementally
                          ──→ Re-fingerprint changed symbols
                          ──→ Persist updated snapshot

                Query
                ─────
  Tool call ──→ Read from shared index (RwLock)
            ──→ Compute results
            ──→ Return JSON response
```

---

## MCP server binary entry point

```rust
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// Entry point for the MCP context server binary.
/// Launched by the orchestrator as: `roko mcp code-intelligence`
pub async fn run_mcp_server(workspace_root: &Path) -> anyhow::Result<()> {
    // Step 1: Load or build the index.
    let index = load_or_build_index(workspace_root).await?;
    let graph = Arc::new(build_symbol_graph(&index)?);
    let server = Arc::new(CodeIntelligenceServer {
        index: Arc::new(index),
        graph,
        config: ServerConfig::from_env()?,
    });

    // Step 2: Start background file watcher for incremental updates.
    let index_ref = Arc::clone(&server.index);
    let watcher_handle = tokio::spawn(async move {
        watch_and_reindex(workspace_root, index_ref).await
    });

    // Step 3: Read JSON-RPC requests from stdin, write responses to stdout.
    let stdin = BufReader::new(tokio::io::stdin());
    let mut stdout = tokio::io::stdout();
    let mut lines = stdin.lines();

    while let Some(line) = lines.next_line().await? {
        let request: JsonRpcRequest = serde_json::from_str(&line)?;

        let response = match request.method.as_str() {
            "tools/list" => server.list_tools(),
            "tools/call" => {
                let tool_name = request.params["name"].as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing tool name"))?;
                let arguments = &request.params["arguments"];
                server.handle_tool_call(tool_name, arguments.clone()).await
            }
            _ => Err(anyhow::anyhow!("Unknown method: {}", request.method)),
        };

        let json_response = match response {
            Ok(result) => serde_json::json!({
                "jsonrpc": "2.0",
                "id": request.id,
                "result": result
            }),
            Err(e) => serde_json::json!({
                "jsonrpc": "2.0",
                "id": request.id,
                "error": { "code": -1, "message": e.to_string() }
            }),
        };

        stdout.write_all(json_response.to_string().as_bytes()).await?;
        stdout.write_all(b"\n").await?;
        stdout.flush().await?;
    }

    watcher_handle.abort();
    Ok(())
}

#[derive(serde::Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: serde_json::Value,
    method: String,
    params: serde_json::Value,
}
```

### Full implementation spec for all 10 MCP tools

Each tool handler validates input, queries the index, and returns structured JSON:

```rust
impl CodeIntelligenceServer {
    /// Tool 1: search_code -- multi-strategy code search.
    pub async fn search_code(&self, params: serde_json::Value) -> Result<serde_json::Value> {
        let query: String = required_field(&params, "query")?;
        let strategy = optional_field(&params, "strategy").unwrap_or("hybrid".into());
        let max_results = optional_field::<u64>(&params, "max_results").unwrap_or(10);
        let file_pattern = optional_field::<String>(&params, "file_pattern");
        let kind_filter = optional_field::<String>(&params, "kind_filter");

        validate_max_results(max_results, 100)?;

        let results = match strategy.as_str() {
            "keyword" => self.index.search_keyword(&query, max_results)?,
            "structural" => self.index.search_structural(&query, max_results)?,
            "hdc" => self.index.search_hdc(&query, max_results)?,
            "embedding" => self.index.search_embedding(&query, max_results)?,
            "hybrid" => self.index.search_rrf(&query, max_results)?,
            _ => anyhow::bail!("Unknown strategy: {}", strategy),
        };

        // Apply filters.
        let filtered = results.into_iter()
            .filter(|r| file_pattern.as_ref().map_or(true, |p| glob_match(p, &r.file)))
            .filter(|r| kind_filter.as_ref().map_or(true, |k| r.kind == *k))
            .collect::<Vec<_>>();

        Ok(serde_json::to_value(filtered)?)
    }

    /// Tool 2: get_symbol_context -- detailed symbol information.
    pub async fn get_symbol_context(&self, params: serde_json::Value) -> Result<serde_json::Value> {
        let name: String = required_field(&params, "symbol_name")?;
        let file_path = optional_field::<String>(&params, "file_path");
        let include_deps = optional_field::<bool>(&params, "include_dependencies").unwrap_or(true);
        let include_callers = optional_field::<bool>(&params, "include_callers").unwrap_or(true);
        let depth = optional_field::<u32>(&params, "expansion_depth").unwrap_or(1);

        let symbol = self.index.resolve_symbol(&name, file_path.as_deref())?;
        let mut context = SymbolContext::from(&symbol);

        if include_deps {
            context.dependencies = self.graph.forward_edges(&symbol.id, depth)?;
        }
        if include_callers {
            context.callers = self.graph.reverse_edges(&symbol.id, depth)?;
        }
        context.pagerank = self.graph.pagerank_score(&symbol.id);

        Ok(serde_json::to_value(context)?)
    }

    /// Tool 3: get_file_ast -- file structure overview.
    pub async fn get_file_ast(&self, params: serde_json::Value) -> Result<serde_json::Value> {
        let file_path: String = required_field(&params, "file_path")?;
        validate_workspace_path(&file_path, &self.config.workspace_root)?;
        let include_bodies = optional_field::<bool>(&params, "include_bodies").unwrap_or(false);
        let symbols = self.index.symbols_in_file(&file_path)?;
        Ok(serde_json::to_value(symbols)?)
    }

    /// Tool 4: find_similar_patterns -- HDC similarity search.
    pub async fn find_similar(&self, params: serde_json::Value) -> Result<serde_json::Value> {
        let reference: String = required_field(&params, "reference")?;
        let min_sim = optional_field::<f64>(&params, "min_similarity").unwrap_or(0.6);
        let max_results = optional_field::<u64>(&params, "max_results").unwrap_or(10);
        let results = self.index.find_similar(&reference, min_sim, max_results)?;
        Ok(serde_json::to_value(results)?)
    }

    /// Tool 5: get_index_stats -- index health and coverage.
    pub async fn get_stats(&self, _params: serde_json::Value) -> Result<serde_json::Value> {
        let stats = self.index.stats()?;
        Ok(serde_json::to_value(stats)?)
    }

    /// Tool 6: find_references -- symbol usage sites.
    pub async fn find_refs(&self, params: serde_json::Value) -> Result<serde_json::Value> {
        let name: String = required_field(&params, "symbol_name")?;
        let file_path = optional_field::<String>(&params, "file_path");
        let include_defs = optional_field::<bool>(&params, "include_definitions").unwrap_or(false);
        let refs = self.graph.references(&name, file_path.as_deref(), include_defs)?;
        Ok(serde_json::to_value(refs)?)
    }

    /// Tool 7: find_implementations -- trait/interface implementations.
    pub async fn find_impls(&self, params: serde_json::Value) -> Result<serde_json::Value> {
        let trait_name: String = required_field(&params, "trait_name")?;
        let include_methods = optional_field::<bool>(&params, "include_methods").unwrap_or(true);
        let impls = self.graph.implementations(&trait_name, include_methods)?;
        Ok(serde_json::to_value(impls)?)
    }

    /// Tool 8: get_callers -- call graph traversal.
    pub async fn get_callers(&self, params: serde_json::Value) -> Result<serde_json::Value> {
        let name: String = required_field(&params, "function_name")?;
        let file_path = optional_field::<String>(&params, "file_path");
        let transitive = optional_field::<bool>(&params, "transitive").unwrap_or(false);
        let max_depth = optional_field::<u32>(&params, "max_depth").unwrap_or(2);
        let callers = self.graph.callers(&name, file_path.as_deref(), transitive, max_depth)?;
        Ok(serde_json::to_value(callers)?)
    }

    /// Tool 9: workspace_map -- structural overview.
    pub async fn workspace_map(&self, params: serde_json::Value) -> Result<serde_json::Value> {
        let depth = optional_field::<String>(&params, "depth").unwrap_or("module".into());
        let focus = optional_field::<String>(&params, "focus_path");
        let map = self.index.workspace_map(&depth, focus.as_deref())?;
        Ok(serde_json::to_value(map)?)
    }

    /// Tool 10: get_context -- automated context assembly.
    pub async fn get_context(&self, params: serde_json::Value) -> Result<serde_json::Value> {
        let task: String = required_field(&params, "task")?;
        let budget = optional_field::<u64>(&params, "token_budget").unwrap_or(40_000);
        let include_tests = optional_field::<bool>(&params, "include_tests").unwrap_or(false);
        validate_token_budget(budget, 200_000)?;
        let context = self.index.assemble_context(&task, budget, include_tests)?;
        Ok(serde_json::to_value(context)?)
    }
}
```

### Index lifecycle management

```rust
/// Load index from snapshot or build fresh.
async fn load_or_build_index(workspace_root: &Path) -> Result<CodeIndex> {
    let db_path = workspace_root.join(".roko/index.db");

    if db_path.exists() {
        // Load existing index, run incremental update.
        let index = CodeIndex::open(&db_path)?;
        let stats = index.incremental_update(workspace_root)?;
        tracing::info!(
            added = stats.added,
            updated = stats.updated,
            unchanged = stats.unchanged,
            "Incremental index update complete"
        );
        Ok(index)
    } else {
        // Full build.
        let index = CodeIndex::create(&db_path)?;
        let stats = index.full_build(workspace_root)?;
        tracing::info!(
            files = stats.added,
            symbols = stats.total_symbols,
            "Full index build complete"
        );
        Ok(index)
    }
}

/// Watch for file changes and re-index incrementally.
async fn watch_and_reindex(
    workspace_root: &Path,
    index: Arc<CodeIndex>,
) -> Result<()> {
    let (tx, mut rx) = tokio::sync::mpsc::channel(256);
    let mut watcher = notify::recommended_watcher(move |event| {
        let _ = tx.blocking_send(event);
    })?;

    watcher.watch(workspace_root, notify::RecursiveMode::Recursive)?;

    // Debounce: wait 500ms after last change before re-indexing.
    let mut last_change = Instant::now();
    let debounce = Duration::from_millis(500);

    loop {
        tokio::select! {
            Some(Ok(_event)) = rx.recv() => {
                last_change = Instant::now();
            }
            _ = tokio::time::sleep(debounce) => {
                if last_change.elapsed() >= debounce {
                    if let Err(e) = index.incremental_update(workspace_root) {
                        tracing::warn!("Re-index failed: {}", e);
                    }
                }
            }
        }
    }
}
```

### Index sharing between concurrent agents

Multiple agents can query the same index concurrently. The MCP server uses `Arc<CodeIndex>` with internal `RwLock` for thread-safe access:

```rust
pub struct CodeIndex {
    /// SQLite connection pool (r2d2).
    /// Multiple readers, one writer (WAL mode).
    pool: r2d2::Pool<r2d2_sqlite::SqliteConnectionManager>,
    /// In-memory caches (symbol graph, fingerprints).
    cache: parking_lot::RwLock<IndexCache>,
}

impl CodeIndex {
    /// Read operations acquire a shared read lock.
    /// Multiple agents can read concurrently.
    pub fn search_rrf(&self, query: &str, limit: u64) -> Result<Vec<SearchResult>> {
        let conn = self.pool.get()?;
        let cache = self.cache.read();
        // ... search logic using conn + cache
    }

    /// Write operations (re-indexing) acquire an exclusive write lock.
    /// Blocks readers briefly during commit.
    pub fn incremental_update(&self, workspace_root: &Path) -> Result<UpdateStats> {
        let conn = self.pool.get()?;
        let mut cache = self.cache.write();
        // ... update logic
    }
}
```

**Configuration:**

```toml
[mcp.code_intelligence]
max_concurrent_queries = 16    # Connection pool size. Range: 1..64.
query_timeout_ms = 5000        # Per-query timeout. Range: 1000..30000.
rate_limit_per_agent = 100     # Queries per minute per agent. Range: 10..1000.
debounce_ms = 500              # File change debounce. Range: 100..5000.
```

---

## Security Considerations

### Input validation

All tool inputs must be validated:
- **File paths** — Must be within the workspace directory. No path traversal (`../../../etc/passwd`).
- **Query strings** — Sanitized for SQL injection if FTS5 is used.
- **Token budgets** — Capped at a maximum to prevent memory exhaustion.
- **Result limits** — Capped at a maximum to prevent response explosion.

### Privacy

The MCP server respects the privacy configuration from the context overlay system:
- Files matching `ignore_files` patterns are never indexed
- Symbols matching `blocked_symbols` are never returned in results
- Content matching `redact_patterns` is stripped from output

### Input validation implementation

```rust
/// Validate that a file path is within the workspace.
fn validate_workspace_path(path: &str, workspace_root: &Path) -> Result<()> {
    let canonical = workspace_root.join(path).canonicalize()
        .map_err(|_| anyhow::anyhow!("Invalid path: {}", path))?;
    if !canonical.starts_with(workspace_root) {
        anyhow::bail!("Path traversal blocked: {} escapes workspace", path);
    }
    Ok(())
}

/// Validate max_results is within bounds.
fn validate_max_results(max: u64, cap: u64) -> Result<()> {
    if max > cap {
        anyhow::bail!("max_results {} exceeds cap {}", max, cap);
    }
    Ok(())
}

/// Validate token budget is within bounds.
fn validate_token_budget(budget: u64, cap: u64) -> Result<()> {
    if budget > cap {
        anyhow::bail!("token_budget {} exceeds cap {}", budget, cap);
    }
    Ok(())
}

/// Sanitize FTS5 query to prevent SQL injection.
/// FTS5 queries use a restricted syntax; strip anything that looks like SQL.
fn sanitize_fts_query(query: &str) -> String {
    // Allow alphanumeric, spaces, quotes, asterisks (prefix search), dashes.
    // Strip semicolons, parentheses used for SQL subqueries, etc.
    query.chars()
        .filter(|c| c.is_alphanumeric() || " \"*-_.:".contains(*c))
        .collect()
}
```

### Rate limiting per agent

```rust
/// Per-agent rate limiter using a sliding window counter.
pub struct AgentRateLimiter {
    /// Limits per agent ID.
    windows: parking_lot::Mutex<HashMap<String, SlidingWindow>>,
    /// Maximum queries per minute per agent.
    max_per_minute: u64,
}

struct SlidingWindow {
    timestamps: VecDeque<Instant>,
}

impl AgentRateLimiter {
    pub fn check(&self, agent_id: &str) -> bool {
        let mut windows = self.windows.lock();
        let window = windows.entry(agent_id.to_string())
            .or_insert_with(|| SlidingWindow { timestamps: VecDeque::new() });

        let one_min_ago = Instant::now() - Duration::from_secs(60);
        while window.timestamps.front().map_or(false, |t| *t < one_min_ago) {
            window.timestamps.pop_front();
        }

        if window.timestamps.len() as u64 >= self.max_per_minute {
            return false;
        }
        window.timestamps.push_back(Instant::now());
        true
    }
}
```

### Test criteria

- `search_code` with strategy "hybrid" returns results ranked by RRF score
- `search_code` with `file_pattern` filters results to matching files
- `get_symbol_context` resolves ambiguous symbol names when `file_path` is provided
- `get_file_ast` rejects paths outside the workspace root (path traversal prevention)
- `get_context` respects `token_budget` -- assembled context does not exceed the budget
- `find_references` returns both definitions and usages when `include_definitions` is true
- `find_implementations` lists all trait implementors with correct method signatures
- `get_callers` with `transitive: true` traverses the call graph to `max_depth`
- `workspace_map` at "crate" depth lists all workspace crates with dependency edges
- Rate limiter blocks the 101st query within a minute from a single agent
- File watcher debounces rapid changes into a single re-index call
- Index loads from snapshot on startup when `.roko/index.db` exists
- Concurrent read queries do not block each other under WAL mode

---

## Comparison with Existing MCP Patterns

### Roko's built-in tools

Roko already has 19 built-in tools in `roko-std`. The MCP context server adds code-intelligence-specific tools that complement the existing set:

| Existing tool | MCP equivalent | Difference |
|---|---|---|
| `file_read` | `get_file_ast` | MCP returns structured AST, not raw text |
| `file_search` | `search_code` | MCP uses multi-strategy search with ranking |
| `grep_search` | `search_code` (keyword mode) | MCP includes structural and similarity search |
| (none) | `get_callers` | New capability: graph-based caller analysis |
| (none) | `find_implementations` | New capability: trait implementation discovery |
| (none) | `workspace_map` | New capability: structural workspace overview |
| (none) | `get_context` | New capability: automated context assembly |

The MCP tools don't replace the built-in tools — they augment them. An agent might use `file_read` for raw file access and `get_symbol_context` for structured code understanding in the same task.

---

## Academic Foundations

- **Model Context Protocol**: Anthropic (2024). Standardized protocol for tool interfaces between LLMs and external services. The transport and schema standard used by the code intelligence server.
- **Aider repository map**: Gauthier (2024). The `workspace_map` tool is directly inspired by Aider's repository map feature, which uses tree-sitter to generate structural overviews of codebases.
- **Language Server Protocol**: Microsoft (2016). LSP tools like `textDocument/references`, `textDocument/implementation`, and `textDocument/definition` inspired the `find_references`, `find_implementations`, and `get_symbol_context` tools. The MCP server provides similar capabilities at higher abstraction.
- **code2seq**: Alon, Brody, Levy, and Yahav (2018), "code2seq: Generating Sequences from Structured Representations of Code." *ICLR*. Demonstrated that structured code representations (AST paths) improve code understanding — motivating the `get_file_ast` tool.

---

## Current Status and Gaps

### Built

- MCP configuration passthrough in `roko.toml` (`agent.mcp_config`)
- MCP config auto-discovery fallback
- Agent dispatch with MCP server support in `roko-agent`
- 19 built-in tools in `roko-std`

### Missing

- MCP context server implementation (no server binary)
- All ten tool implementations
- Index lifecycle management (no startup, no file watching, no snapshot persistence)
- Input validation and security layer
- Rate limiting
- Privacy/redaction integration
- Integration tests with agent dispatch

---

## Cross-References

- See [06-context-assembly-from-code.md](./06-context-assembly-from-code.md) for the context assembly pipeline that `get_context` triggers
- See [08-index-db-scaling.md](./08-index-db-scaling.md) for the storage backend that the server queries
- See [10-current-status-and-gaps.md](./10-current-status-and-gaps.md) for the implementation roadmap
- See topic [18-tools](../18-tools/INDEX.md) for the broader tool system architecture
- See topic [01-orchestration](../01-orchestration/INDEX.md) for how the orchestrator manages MCP server processes
