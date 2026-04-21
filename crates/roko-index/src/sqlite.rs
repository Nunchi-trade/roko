//! Optional SQLite-backed persistent code index.
//!
//! Feature-gated behind `sqlite`. Stores symbols, edges, and file metadata in
//! a local `.roko/index.db` file with WAL mode for concurrent reads.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context as _, Result};
use rusqlite::{Connection, params};

use crate::graph::EdgeKind;
use crate::symbol::SymbolId;
use crate::workspace::SymbolInfo;
use roko_core::language::{SymbolKind, Visibility};

/// Statistics returned from an incremental update.
#[derive(Clone, Debug, Default)]
pub struct UpdateStats {
    /// Number of files that were re-parsed.
    pub files_updated: usize,
    /// Number of files that were skipped (unchanged).
    pub files_skipped: usize,
    /// Number of symbols inserted or replaced.
    pub symbols_upserted: usize,
    /// Number of edges inserted or replaced.
    pub edges_upserted: usize,
}

/// SQLite-backed persistent index for symbols and edges.
pub struct SqliteIndex {
    conn: Connection,
}

impl SqliteIndex {
    /// Open (or create) the index database at `path`.
    ///
    /// Enables WAL mode for concurrent reads.
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)
            .with_context(|| format!("opening index db at {}", path.display()))?;

        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;

        let index = Self { conn };
        index.create_tables()?;
        Ok(index)
    }

    /// Open an in-memory database (useful for testing).
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let index = Self { conn };
        index.create_tables()?;
        Ok(index)
    }

    /// Create the schema tables if they do not already exist.
    pub fn create_tables(&self) -> Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS files (
                path     TEXT PRIMARY KEY,
                mtime_ns INTEGER NOT NULL,
                hash     TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS symbols (
                id         INTEGER PRIMARY KEY AUTOINCREMENT,
                file_path  TEXT NOT NULL,
                name       TEXT NOT NULL,
                kind       TEXT NOT NULL,
                line       INTEGER NOT NULL,
                col        INTEGER NOT NULL DEFAULT 0,
                visibility TEXT NOT NULL DEFAULT 'Private',
                UNIQUE(file_path, name, kind)
            );
            CREATE INDEX IF NOT EXISTS idx_symbols_name ON symbols(name);
            CREATE INDEX IF NOT EXISTS idx_symbols_file ON symbols(file_path);

            CREATE TABLE IF NOT EXISTS edges (
                from_file TEXT NOT NULL,
                from_name TEXT NOT NULL,
                from_kind TEXT NOT NULL,
                to_file   TEXT NOT NULL,
                to_name   TEXT NOT NULL,
                to_kind   TEXT NOT NULL,
                edge_kind TEXT NOT NULL,
                UNIQUE(from_file, from_name, from_kind, to_file, to_name, to_kind, edge_kind)
            );
            CREATE INDEX IF NOT EXISTS idx_edges_from ON edges(from_file, from_name);
            CREATE INDEX IF NOT EXISTS idx_edges_to ON edges(to_file, to_name);",
        )?;
        self.create_fts_table()?;
        Ok(())
    }

    /// Create the FTS5 virtual table for full-text keyword search (CODE-06).
    ///
    /// Uses a standalone (non-content-sync) FTS table populated via `rebuild_fts`.
    fn create_fts_table(&self) -> Result<()> {
        self.conn.execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS symbols_fts USING fts5(
                name,
                file_path,
                kind,
                sym_id
            );",
        )?;
        Ok(())
    }

    /// Rebuild the FTS5 index from the current symbols table.
    ///
    /// Should be called after bulk inserts or an incremental update.
    pub fn rebuild_fts(&self) -> Result<()> {
        self.conn.execute_batch(
            "DELETE FROM symbols_fts;
             INSERT INTO symbols_fts(sym_id, name, file_path, kind)
                SELECT id, name, file_path, kind FROM symbols;",
        )?;
        Ok(())
    }

    /// Full-text keyword search using FTS5 (CODE-06).
    ///
    /// Searches symbol names and file paths using the SQLite FTS5 engine.
    /// Returns up to 100 matching symbols sorted by FTS5 relevance rank.
    pub fn fts_search(&self, query: &str) -> Result<Vec<SymbolInfo>> {
        // Escape special FTS5 characters and add prefix matching.
        let safe_query = query.replace('"', "\"\"");
        let fts_query = format!("\"{safe_query}\"*");

        let mut stmt = self.conn.prepare(
            "SELECT s.file_path, s.name, s.kind, s.line, s.visibility
             FROM symbols_fts fts
             JOIN symbols s ON s.id = CAST(fts.sym_id AS INTEGER)
             WHERE symbols_fts MATCH ?1
             ORDER BY rank
             LIMIT 100",
        )?;

        let rows = stmt.query_map(params![fts_query], |row| {
            let file_path: String = row.get(0)?;
            let name: String = row.get(1)?;
            let kind_str: String = row.get(2)?;
            let line: usize = row.get(3)?;
            let vis_str: String = row.get(4)?;
            Ok((file_path, name, kind_str, line, vis_str))
        })?;

        let mut results = Vec::new();
        for row in rows {
            let (file_path, name, kind_str, line, vis_str) = row?;
            let kind = parse_symbol_kind(&kind_str);
            let visibility = parse_visibility(&vis_str);
            results.push(SymbolInfo {
                id: SymbolId::new(file_path, name, kind),
                visibility,
                line,
                language: String::new(),
            });
        }
        Ok(results)
    }

    /// Insert or replace a symbol.
    pub fn insert_symbol(&self, symbol: &SymbolInfo) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO symbols (file_path, name, kind, line, col, visibility)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                symbol.id.file_path,
                symbol.id.symbol_name,
                format!("{:?}", symbol.id.kind),
                symbol.line,
                0,
                format!("{:?}", symbol.visibility),
            ],
        )?;
        Ok(())
    }

    /// Insert or replace an edge.
    pub fn insert_edge(&self, from: &SymbolId, to: &SymbolId, kind: &EdgeKind) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO edges (from_file, from_name, from_kind, to_file, to_name, to_kind, edge_kind)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                from.file_path,
                from.symbol_name,
                format!("{:?}", from.kind),
                to.file_path,
                to.symbol_name,
                format!("{:?}", to.kind),
                format!("{:?}", kind),
            ],
        )?;
        Ok(())
    }

    /// Query symbols whose name contains the query string (case-insensitive).
    pub fn query_symbols(&self, query: &str) -> Result<Vec<SymbolInfo>> {
        let pattern = format!("%{query}%");
        let mut stmt = self.conn.prepare(
            "SELECT file_path, name, kind, line, visibility FROM symbols
             WHERE name LIKE ?1
             ORDER BY name, file_path
             LIMIT 100",
        )?;

        let rows = stmt.query_map(params![pattern], |row| {
            let file_path: String = row.get(0)?;
            let name: String = row.get(1)?;
            let kind_str: String = row.get(2)?;
            let line: usize = row.get(3)?;
            let vis_str: String = row.get(4)?;

            Ok((file_path, name, kind_str, line, vis_str))
        })?;

        let mut results = Vec::new();
        for row in rows {
            let (file_path, name, kind_str, line, vis_str) = row?;
            let kind = parse_symbol_kind(&kind_str);
            let visibility = parse_visibility(&vis_str);
            results.push(SymbolInfo {
                id: SymbolId::new(file_path, name, kind),
                visibility,
                line,
                language: String::new(),
            });
        }
        Ok(results)
    }

    /// Perform an incremental update: check file mtimes and only re-index
    /// changed files.
    ///
    /// `changed_files` provides the set of files to check. Files whose mtime
    /// has not changed (compared to the stored value) are skipped.
    pub fn incremental_update<F>(
        &self,
        changed_files: &[PathBuf],
        mut index_file: F,
    ) -> Result<UpdateStats>
    where
        F: FnMut(&Path) -> Result<(Vec<SymbolInfo>, Vec<(SymbolId, SymbolId, EdgeKind)>)>,
    {
        let mut stats = UpdateStats::default();
        let tx = self.conn.unchecked_transaction()?;

        for file_path in changed_files {
            let path_str = file_path.to_string_lossy().to_string();

            let current_mtime = file_mtime_ns(file_path);
            let stored_mtime: Option<i64> = tx
                .query_row(
                    "SELECT mtime_ns FROM files WHERE path = ?1",
                    params![path_str],
                    |row| row.get(0),
                )
                .ok();

            if stored_mtime == Some(current_mtime) {
                stats.files_skipped += 1;
                continue;
            }

            // Remove stale data for this file.
            tx.execute(
                "DELETE FROM symbols WHERE file_path = ?1",
                params![path_str],
            )?;
            tx.execute(
                "DELETE FROM edges WHERE from_file = ?1 OR to_file = ?1",
                params![path_str],
            )?;

            // Re-index.
            let (symbols, edges) = index_file(file_path)?;

            for sym in &symbols {
                tx.execute(
                    "INSERT OR REPLACE INTO symbols (file_path, name, kind, line, col, visibility)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        sym.id.file_path,
                        sym.id.symbol_name,
                        format!("{:?}", sym.id.kind),
                        sym.line,
                        0,
                        format!("{:?}", sym.visibility),
                    ],
                )?;
                stats.symbols_upserted += 1;
            }

            for (from, to, kind) in &edges {
                tx.execute(
                    "INSERT OR REPLACE INTO edges (from_file, from_name, from_kind, to_file, to_name, to_kind, edge_kind)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        from.file_path,
                        from.symbol_name,
                        format!("{:?}", from.kind),
                        to.file_path,
                        to.symbol_name,
                        format!("{:?}", to.kind),
                        format!("{:?}", kind),
                    ],
                )?;
                stats.edges_upserted += 1;
            }

            // Update file record.
            let hash = blake3::hash(path_str.as_bytes()).to_hex().to_string();
            tx.execute(
                "INSERT OR REPLACE INTO files (path, mtime_ns, hash) VALUES (?1, ?2, ?3)",
                params![path_str, current_mtime, hash],
            )?;

            stats.files_updated += 1;
        }

        tx.commit()?;
        Ok(stats)
    }

    /// Total number of symbols stored.
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    pub fn symbol_count(&self) -> Result<usize> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM symbols", [], |row| row.get(0))?;
        Ok(count.max(0) as usize)
    }

    /// Total number of edges stored.
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    pub fn edge_count(&self) -> Result<usize> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM edges", [], |row| row.get(0))?;
        Ok(count.max(0) as usize)
    }
}

#[allow(clippy::cast_possible_truncation)]
fn file_mtime_ns(path: &Path) -> i64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map_or(0, |d| {
            let ns = d.as_nanos();
            if ns > i64::MAX as u128 {
                i64::MAX
            } else {
                ns as i64
            }
        })
}

#[allow(clippy::match_same_arms)]
fn parse_symbol_kind(s: &str) -> SymbolKind {
    match s {
        "Function" => SymbolKind::Function,
        "Struct" => SymbolKind::Struct,
        "Enum" => SymbolKind::Enum,
        "Trait" => SymbolKind::Trait,
        "Const" => SymbolKind::Const,
        "Type" => SymbolKind::Type,
        "Module" => SymbolKind::Module,
        "Impl" => SymbolKind::Impl,
        _ => SymbolKind::Function,
    }
}

fn parse_visibility(s: &str) -> Visibility {
    match s {
        "Public" => Visibility::Public,
        _ => Visibility::Private,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_symbol(file: &str, name: &str, kind: SymbolKind, line: usize) -> SymbolInfo {
        SymbolInfo {
            id: SymbolId::new(file, name, kind),
            visibility: Visibility::Public,
            line,
            language: "rust".into(),
        }
    }

    #[test]
    fn open_and_insert_symbols() {
        let db = SqliteIndex::open_in_memory().expect("open in-memory db");

        let sym = test_symbol("lib.rs", "main", SymbolKind::Function, 1);
        db.insert_symbol(&sym).expect("insert symbol");

        let results = db.query_symbols("main").expect("query");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id.symbol_name, "main");
        assert_eq!(results[0].line, 1);
    }

    #[test]
    fn insert_and_count_edges() {
        let db = SqliteIndex::open_in_memory().expect("open in-memory db");

        let from = SymbolId::new("a.rs", "foo", SymbolKind::Function);
        let to = SymbolId::new("b.rs", "Bar", SymbolKind::Struct);
        db.insert_edge(&from, &to, &EdgeKind::TypeRef)
            .expect("insert edge");

        assert_eq!(db.edge_count().expect("edge count"), 1);
    }

    #[test]
    fn query_is_case_insensitive() {
        let db = SqliteIndex::open_in_memory().expect("open in-memory db");

        let sym = test_symbol("lib.rs", "MyStruct", SymbolKind::Struct, 10);
        db.insert_symbol(&sym).expect("insert");

        let results = db.query_symbols("mystruct").expect("query");
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn fts_search_finds_symbols() {
        let db = SqliteIndex::open_in_memory().expect("open in-memory db");

        let sym1 = test_symbol("lib.rs", "process_request", SymbolKind::Function, 10);
        let sym2 = test_symbol("lib.rs", "handle_response", SymbolKind::Function, 20);
        let sym3 = test_symbol("types.rs", "RequestConfig", SymbolKind::Struct, 5);
        db.insert_symbol(&sym1).expect("insert");
        db.insert_symbol(&sym2).expect("insert");
        db.insert_symbol(&sym3).expect("insert");
        db.rebuild_fts().expect("rebuild fts");

        let results = db.fts_search("request").expect("fts search");
        assert!(!results.is_empty());
        let names: Vec<&str> = results.iter().map(|s| s.id.symbol_name.as_str()).collect();
        assert!(
            names.contains(&"process_request") || names.contains(&"RequestConfig"),
            "expected FTS to find request-related symbols, got {names:?}"
        );
    }

    #[test]
    fn fts_search_empty_index() {
        let db = SqliteIndex::open_in_memory().expect("open in-memory db");
        db.rebuild_fts().expect("rebuild fts");
        let results = db.fts_search("anything").expect("fts search");
        assert!(results.is_empty());
    }

    #[test]
    fn incremental_update_skips_unchanged() {
        let db = SqliteIndex::open_in_memory().expect("open in-memory db");

        // Use a non-existent path so mtime will be 0, matching the empty DB.
        let fake_path = PathBuf::from("/nonexistent/file.rs");

        // First pass: file not in DB, so index_file is called.
        let stats = db
            .incremental_update(&[fake_path.clone()], |_path| {
                Ok((
                    vec![test_symbol(
                        "/nonexistent/file.rs",
                        "func",
                        SymbolKind::Function,
                        1,
                    )],
                    vec![],
                ))
            })
            .expect("first update");
        assert_eq!(stats.files_updated, 1);
        assert_eq!(stats.symbols_upserted, 1);

        // Second pass: mtime is still 0 (file doesn't exist), DB has 0 — should skip.
        let stats2 = db
            .incremental_update(&[fake_path], |_path| {
                panic!("should not be called for unchanged file");
            })
            .expect("second update");
        assert_eq!(stats2.files_skipped, 1);
        assert_eq!(stats2.files_updated, 0);
    }
}
