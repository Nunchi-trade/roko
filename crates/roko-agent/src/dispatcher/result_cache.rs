//! Cross-turn tool result cache for deterministic tools (AGT-10).
//!
//! Caches results of deterministic tool calls (Read, Glob, Grep) keyed by
//! `(tool_name, arguments_hash)` with TTL-based invalidation. Write/Edit
//! operations invalidate cached entries for affected paths.
//!
//! This reduces token cost by avoiding redundant tool calls within an agent
//! session and speeds up multi-turn agent loops.

use roko_core::defaults::{DEFAULT_MAX_CACHE_ENTRIES, DEFAULT_RESULT_CACHE_TTL_SECS};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::time::{Duration, Instant};

use roko_core::tool::{ToolCall, ToolResult};

/// Default TTL for cached tool results (5 minutes).
pub const DEFAULT_CACHE_TTL: Duration = Duration::from_secs(DEFAULT_RESULT_CACHE_TTL_SECS);

/// Maximum number of entries before LRU eviction triggers.
const MAX_CACHE_ENTRIES: usize = DEFAULT_MAX_CACHE_ENTRIES;

/// Tools whose results are deterministic for the same arguments (within a
/// session where no writes have occurred to the relevant paths).
const DETERMINISTIC_TOOLS: &[&str] = &[
    "read_file",
    "read",
    "glob",
    "grep",
    "list_directory",
    "search_files",
    "find_definition",
    "find_references",
    "get_symbols",
];

/// Tools whose execution may invalidate cached results.
const INVALIDATING_TOOLS: &[&str] = &[
    "write_file",
    "write",
    "edit_file",
    "edit",
    "create_file",
    "delete_file",
    "bash",
    "execute_command",
    "shell",
];

/// A cache key derived from the tool name and a hash of its arguments.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct CacheKey {
    tool_name: String,
    args_hash: u64,
}

/// A cached tool result with metadata for TTL and LRU.
struct CacheEntry {
    result: ToolResult,
    created_at: Instant,
    last_accessed: Instant,
    /// File paths referenced by this tool call (for invalidation).
    affected_paths: Vec<String>,
}

/// Cross-turn tool result cache.
///
/// Thread-safe: designed to be shared via `Arc<ToolResultCache>` but uses
/// interior `&mut self` methods since the dispatcher already serializes
/// access per-agent.
pub struct ToolResultCache {
    entries: HashMap<CacheKey, CacheEntry>,
    ttl: Duration,
    hits: u64,
    misses: u64,
}

impl ToolResultCache {
    /// Create a new cache with the default TTL.
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            ttl: DEFAULT_CACHE_TTL,
            hits: 0,
            misses: 0,
        }
    }

    /// Create a cache with a custom TTL.
    #[must_use]
    pub fn with_ttl(ttl: Duration) -> Self {
        Self {
            entries: HashMap::new(),
            ttl,
            hits: 0,
            misses: 0,
        }
    }

    /// Look up a cached result for the given tool call.
    ///
    /// Returns `Some(result)` on cache hit (tool is deterministic, entry
    /// exists, and TTL has not expired). Returns `None` on miss.
    pub fn get(&mut self, call: &ToolCall) -> Option<ToolResult> {
        if !is_deterministic(&call.name) {
            return None;
        }

        let key = cache_key(call);
        let now = Instant::now();

        // Check if entry exists.
        let entry = match self.entries.get_mut(&key) {
            Some(e) => e,
            None => {
                self.misses += 1;
                return None;
            }
        };

        // Check TTL.
        if now.duration_since(entry.created_at) > self.ttl {
            self.entries.remove(&key);
            self.misses += 1;
            return None;
        }

        entry.last_accessed = now;
        self.hits += 1;
        Some(entry.result.clone())
    }

    /// Store a tool result in the cache.
    ///
    /// Only caches results for deterministic tools. Non-deterministic tools
    /// are silently ignored.
    pub fn put(&mut self, call: &ToolCall, result: ToolResult) {
        if !is_deterministic(&call.name) {
            return;
        }

        // Only cache successful results.
        if matches!(result, ToolResult::Err(_)) {
            return;
        }

        // Evict expired entries if we're at capacity.
        if self.entries.len() >= MAX_CACHE_ENTRIES {
            self.evict_expired_or_lru();
        }

        let key = cache_key(call);
        let paths = extract_paths(call);
        let now = Instant::now();

        self.entries.insert(
            key,
            CacheEntry {
                result,
                created_at: now,
                last_accessed: now,
                affected_paths: paths,
            },
        );
    }

    /// Invalidate cache entries affected by a write operation.
    ///
    /// Called when a write/edit tool modifies a file path. Removes all
    /// cached entries whose `affected_paths` overlap with the modified path.
    pub fn invalidate(&mut self, call: &ToolCall) {
        if !is_invalidating(&call.name) {
            return;
        }

        let modified_paths = extract_paths(call);
        if modified_paths.is_empty() {
            // Bash/shell commands could affect anything -- clear entire cache.
            if call.name == "bash" || call.name == "execute_command" || call.name == "shell" {
                self.entries.clear();
            }
            return;
        }

        self.entries.retain(|_key, entry| {
            !entry
                .affected_paths
                .iter()
                .any(|p| modified_paths.iter().any(|mp| paths_overlap(p, mp)))
        });
    }

    /// Clear all cached entries.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Number of entries currently in the cache.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the cache is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Cache hit count since creation.
    #[must_use]
    pub fn hits(&self) -> u64 {
        self.hits
    }

    /// Cache miss count since creation.
    #[must_use]
    pub fn misses(&self) -> u64 {
        self.misses
    }

    /// Hit rate as a fraction in `[0.0, 1.0]`.
    #[must_use]
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            self.hits as f64 / total as f64
        }
    }

    /// Evict expired entries first, then LRU if still over capacity.
    fn evict_expired_or_lru(&mut self) {
        let now = Instant::now();

        // Pass 1: remove expired.
        self.entries
            .retain(|_, entry| now.duration_since(entry.created_at) <= self.ttl);

        // Pass 2: if still over capacity, remove the least recently accessed.
        while self.entries.len() >= MAX_CACHE_ENTRIES {
            let oldest_key = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_accessed)
                .map(|(key, _)| key.clone());
            if let Some(key) = oldest_key {
                self.entries.remove(&key);
            } else {
                break;
            }
        }
    }
}

impl Default for ToolResultCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Check whether a tool is deterministic (safe to cache).
fn is_deterministic(tool_name: &str) -> bool {
    DETERMINISTIC_TOOLS
        .iter()
        .any(|&t| t == tool_name.to_ascii_lowercase())
}

/// Check whether a tool call might invalidate cached results.
fn is_invalidating(tool_name: &str) -> bool {
    INVALIDATING_TOOLS
        .iter()
        .any(|&t| t == tool_name.to_ascii_lowercase())
}

/// Compute a cache key from a tool call.
fn cache_key(call: &ToolCall) -> CacheKey {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    // Hash the canonical JSON representation of arguments.
    let args_str = call.arguments.to_string();
    args_str.hash(&mut hasher);
    CacheKey {
        tool_name: call.name.to_ascii_lowercase(),
        args_hash: hasher.finish(),
    }
}

/// Extract file paths from a tool call's arguments.
fn extract_paths(call: &ToolCall) -> Vec<String> {
    let mut paths = Vec::new();

    // Common argument names for file paths across tools.
    for key in &["file_path", "path", "paths", "directory", "target"] {
        if let Some(val) = call.arguments.get(key) {
            match val {
                serde_json::Value::String(s) => paths.push(s.clone()),
                serde_json::Value::Array(arr) => {
                    for item in arr {
                        if let serde_json::Value::String(s) = item {
                            paths.push(s.clone());
                        }
                    }
                }
                _ => {}
            }
        }
    }

    paths
}

/// Check whether two paths overlap (one is a prefix of the other, or equal).
fn paths_overlap(a: &str, b: &str) -> bool {
    a == b || a.starts_with(b) || b.starts_with(a)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_call(name: &str, args: serde_json::Value) -> ToolCall {
        ToolCall {
            id: "test-1".into(),
            name: name.into(),
            arguments: args,
            request_ts_ms: 0,
        }
    }

    fn ok_result(content: &str) -> ToolResult {
        ToolResult::Ok {
            content: content.into(),
            is_structured: false,
            artifacts: vec![],
        }
    }

    #[test]
    fn deterministic_tool_detection() {
        assert!(is_deterministic("read_file"));
        assert!(is_deterministic("Read_File")); // case insensitive
        assert!(is_deterministic("glob"));
        assert!(is_deterministic("grep"));
        assert!(!is_deterministic("bash"));
        assert!(!is_deterministic("write_file"));
        assert!(!is_deterministic("unknown_tool"));
    }

    #[test]
    fn invalidating_tool_detection() {
        assert!(is_invalidating("write_file"));
        assert!(is_invalidating("edit_file"));
        assert!(is_invalidating("bash"));
        assert!(!is_invalidating("read_file"));
        assert!(!is_invalidating("glob"));
    }

    #[test]
    fn cache_hit_and_miss() {
        let mut cache = ToolResultCache::new();
        let call = make_call("read_file", json!({"file_path": "/tmp/test.rs"}));

        // Miss on first access.
        assert!(cache.get(&call).is_none());
        assert_eq!(cache.misses(), 1);
        assert_eq!(cache.hits(), 0);

        // Put and hit.
        cache.put(&call, ok_result("file contents"));
        assert!(cache.get(&call).is_some());
        assert_eq!(cache.hits(), 1);
    }

    #[test]
    fn non_deterministic_tools_not_cached() {
        let mut cache = ToolResultCache::new();
        let call = make_call("bash", json!({"command": "ls"}));

        cache.put(&call, ok_result("output"));
        assert!(cache.get(&call).is_none());
        assert!(cache.is_empty());
    }

    #[test]
    fn error_results_not_cached() {
        let mut cache = ToolResultCache::new();
        let call = make_call("read_file", json!({"file_path": "/tmp/missing.rs"}));

        cache.put(
            &call,
            ToolResult::Err(roko_core::tool::ToolError::Other("not found".into())),
        );
        assert!(cache.is_empty());
    }

    #[test]
    fn write_invalidates_read_cache() {
        let mut cache = ToolResultCache::new();
        let read = make_call("read_file", json!({"file_path": "/tmp/test.rs"}));
        let write = make_call("write_file", json!({"file_path": "/tmp/test.rs"}));

        cache.put(&read, ok_result("original"));
        assert_eq!(cache.len(), 1);

        cache.invalidate(&write);
        assert!(cache.is_empty());
    }

    #[test]
    fn bash_invalidates_entire_cache() {
        let mut cache = ToolResultCache::new();
        let read1 = make_call("read_file", json!({"file_path": "/tmp/a.rs"}));
        let read2 = make_call("read_file", json!({"file_path": "/tmp/b.rs"}));

        cache.put(&read1, ok_result("a"));
        cache.put(&read2, ok_result("b"));
        assert_eq!(cache.len(), 2);

        let bash = make_call("bash", json!({"command": "cargo build"}));
        cache.invalidate(&bash);
        assert!(cache.is_empty());
    }

    #[test]
    fn ttl_expiration() {
        let mut cache = ToolResultCache::with_ttl(Duration::from_millis(1));
        let call = make_call("read_file", json!({"file_path": "/tmp/test.rs"}));
        cache.put(&call, ok_result("content"));

        // Sleep briefly to expire the entry.
        std::thread::sleep(Duration::from_millis(5));

        assert!(cache.get(&call).is_none());
    }

    #[test]
    fn different_args_different_entries() {
        let mut cache = ToolResultCache::new();
        let call1 = make_call("read_file", json!({"file_path": "/tmp/a.rs"}));
        let call2 = make_call("read_file", json!({"file_path": "/tmp/b.rs"}));

        cache.put(&call1, ok_result("a"));
        cache.put(&call2, ok_result("b"));
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn hit_rate_calculation() {
        let mut cache = ToolResultCache::new();
        let call = make_call("read_file", json!({"file_path": "/tmp/test.rs"}));

        // One miss.
        cache.get(&call);
        assert_eq!(cache.hit_rate(), 0.0);

        // One hit.
        cache.put(&call, ok_result("content"));
        cache.get(&call);
        assert!((cache.hit_rate() - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn path_overlap_detection() {
        assert!(paths_overlap("/tmp/test.rs", "/tmp/test.rs"));
        assert!(paths_overlap("/tmp/", "/tmp/test.rs"));
        assert!(paths_overlap("/tmp/test.rs", "/tmp/"));
        assert!(!paths_overlap("/tmp/a.rs", "/tmp/b.rs"));
    }

    #[test]
    fn clear_resets_cache() {
        let mut cache = ToolResultCache::new();
        let call = make_call("read_file", json!({"file_path": "/tmp/test.rs"}));
        cache.put(&call, ok_result("content"));
        assert_eq!(cache.len(), 1);

        cache.clear();
        assert!(cache.is_empty());
    }
}
