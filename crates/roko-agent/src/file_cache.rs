//! File-backed response cache for demo determinism.
//!
//! Persists cached LLM responses to disk as JSON so they survive across
//! process restarts. Activated by `ROKO_DEMO_CACHE=1` or `--demo-cache`.
//!
//! Cache key: blake3 hash of (scope + endpoint + body).
//! Cache value: JSON file at `.roko/demo-cache/{hash}.json`.

use std::path::{Path, PathBuf};

use roko_core::ContentHash;
use serde::{Deserialize, Serialize};

/// A cached response stored on disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileCacheEntry {
    /// The response text content.
    pub response_text: String,
    /// Model that generated the response.
    pub model: String,
    /// Input token count.
    pub input_tokens: u32,
    /// Output token count.
    pub output_tokens: u32,
    /// Cost in USD.
    pub cost_usd: f64,
    /// When the entry was cached (Unix timestamp ms).
    pub cached_at_ms: u64,
}

/// File-backed response cache.
pub struct FileCache {
    dir: PathBuf,
}

impl FileCache {
    /// Open (or create) a file cache at the given directory.
    pub fn open(dir: impl Into<PathBuf>) -> Self {
        let dir = dir.into();
        let _ = std::fs::create_dir_all(&dir);
        Self { dir }
    }

    /// Open the default cache at `.roko/demo-cache/` relative to workdir.
    pub fn open_default(workdir: &Path) -> Self {
        Self::open(workdir.join(".roko").join("demo-cache"))
    }

    /// Check if the file cache is enabled via environment variable.
    pub fn is_enabled() -> bool {
        std::env::var_os("ROKO_DEMO_CACHE").is_some_and(|v| v == "1" || v == "true")
    }

    /// Look up a cached response by hash.
    pub fn get(&self, key: &ContentHash) -> Option<FileCacheEntry> {
        let path = self.entry_path(key);
        let data = std::fs::read_to_string(&path).ok()?;
        serde_json::from_str(&data).ok()
    }

    /// Store a response in the cache.
    pub fn put(&self, key: &ContentHash, entry: &FileCacheEntry) -> std::io::Result<()> {
        let path = self.entry_path(key);
        let json = serde_json::to_string_pretty(entry)
            .map_err(std::io::Error::other)?;
        std::fs::write(path, json)
    }

    /// Number of entries in the cache.
    pub fn len(&self) -> usize {
        std::fs::read_dir(&self.dir)
            .map(|entries| entries.filter(|e| e.is_ok()).count())
            .unwrap_or(0)
    }

    /// Whether the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// List all cache entry hashes.
    pub fn keys(&self) -> Vec<ContentHash> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        entries
            .filter_map(|e| {
                let e = e.ok()?;
                let name = e.file_name().to_string_lossy().replace(".json", "");
                ContentHash::from_hex(&name)
            })
            .collect()
    }

    /// Clear all cached entries.
    pub fn clear(&self) -> std::io::Result<()> {
        if self.dir.exists() {
            std::fs::remove_dir_all(&self.dir)?;
            std::fs::create_dir_all(&self.dir)?;
        }
        Ok(())
    }

    fn entry_path(&self, key: &ContentHash) -> PathBuf {
        self.dir.join(format!("{}.json", key.to_hex()))
    }
}

/// Compute a cache key from request parameters.
pub fn cache_key(scope: &str, endpoint: &str, body: &[u8]) -> ContentHash {
    super::cache::request_hash(scope, endpoint, body)
}

/// Compute a cache key from a prompt string and model name.
pub fn prompt_cache_key(model: &str, prompt: &str) -> ContentHash {
    cache_key(model, "prompt", prompt.as_bytes())
}

/// Get the global file cache instance (lazily initialized).
///
/// Uses the current working directory's `.roko/demo-cache/` by default.
/// Only active when `ROKO_DEMO_CACHE=1` is set.
pub fn global_file_cache() -> Option<FileCache> {
    if !FileCache::is_enabled() {
        return None;
    }
    let workdir = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    Some(FileCache::open_default(&workdir))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn roundtrip() {
        let tmp = TempDir::new().unwrap();
        let cache = FileCache::open(tmp.path().join("cache"));
        let key = ContentHash::of(b"test-prompt");

        assert!(cache.get(&key).is_none());
        assert!(cache.is_empty());

        let entry = FileCacheEntry {
            response_text: "Hello world".into(),
            model: "haiku".into(),
            input_tokens: 100,
            output_tokens: 50,
            cost_usd: 0.001,
            cached_at_ms: 1234567890,
        };
        cache.put(&key, &entry).unwrap();

        assert_eq!(cache.len(), 1);
        let got = cache.get(&key).unwrap();
        assert_eq!(got.response_text, "Hello world");
        assert_eq!(got.model, "haiku");
    }

    #[test]
    fn clear_removes_entries() {
        let tmp = TempDir::new().unwrap();
        let cache = FileCache::open(tmp.path().join("cache"));
        let key = ContentHash::of(b"test");
        let entry = FileCacheEntry {
            response_text: "x".into(),
            model: "m".into(),
            input_tokens: 0,
            output_tokens: 0,
            cost_usd: 0.0,
            cached_at_ms: 0,
        };
        cache.put(&key, &entry).unwrap();
        assert_eq!(cache.len(), 1);
        cache.clear().unwrap();
        assert!(cache.is_empty());
    }

    #[test]
    fn keys_lists_entries() {
        let tmp = TempDir::new().unwrap();
        let cache = FileCache::open(tmp.path().join("cache"));
        let entry = FileCacheEntry {
            response_text: "x".into(),
            model: "m".into(),
            input_tokens: 0,
            output_tokens: 0,
            cost_usd: 0.0,
            cached_at_ms: 0,
        };
        let k1 = ContentHash::of(b"a");
        let k2 = ContentHash::of(b"b");
        cache.put(&k1, &entry).unwrap();
        cache.put(&k2, &entry).unwrap();
        assert_eq!(cache.keys().len(), 2);
    }
}
