//! Memory pointers for large tool results (§36.r, parity items 36.124–36.128).
//!
//! **Why**: Memory Pointers (arXiv:2511.22729) — LLMs interact with
//! *pointers* to large data, not the raw bytes. MemTool achieved 0.90
//! tool-calling accuracy under **hard context limits** via hybrid
//! pruning. Roko's local-model backends (8K–32K contexts) cannot afford
//! to inline a 40 kB `read_file` result into the next turn.
//!
//! This module defines the canonical [`MemoryPointer`] type. The pointer
//! store + GC live in `roko-agent::pointer` (concrete runtime code).
//! The `expand_pointer` meta-tool (§36.126) lives in `roko-std::tool`.

use serde::{Deserialize, Serialize};

// ─── MemoryPointer ────────────────────────────────────────────────────────

/// A reference to a large tool-result payload, held in the pointer store.
///
/// When a tool's result exceeds `max_inline_bytes` (default 4 kB), the
/// dispatcher stores the full payload and returns a [`MemoryPointer`]
/// instead. The LLM can then invoke `expand_pointer(id, range?)` to
/// fetch the full content (or a range) on demand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryPointer {
    /// Stable identifier. Implementations typically use BLAKE3 of the
    /// payload + trace_id to make pointers content-addressed.
    pub id: String,
    /// Small leading preview (≤256 bytes) so the LLM has enough signal
    /// to decide whether to expand. Truncated at the nearest char
    /// boundary for UTF-8 safety.
    pub preview: String,
    /// Size of the full payload in bytes.
    pub size_bytes: u64,
    /// IANA MIME type.
    pub mime_type: String,
    /// Creation timestamp (ms since epoch). Used by the GC to evict.
    pub created_ms: i64,
}

impl MemoryPointer {
    /// Maximum length of [`Self::preview`] in bytes.
    pub const MAX_PREVIEW_BYTES: usize = 256;

    /// Construct a pointer, truncating `preview` safely to
    /// [`Self::MAX_PREVIEW_BYTES`].
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        preview: &str,
        size_bytes: u64,
        mime_type: impl Into<String>,
        created_ms: i64,
    ) -> Self {
        let preview = if preview.len() <= Self::MAX_PREVIEW_BYTES {
            preview.to_owned()
        } else {
            // Find the last char boundary ≤ MAX_PREVIEW_BYTES.
            let mut idx = Self::MAX_PREVIEW_BYTES;
            while !preview.is_char_boundary(idx) && idx > 0 {
                idx -= 1;
            }
            preview[..idx].to_owned()
        };
        Self {
            id: id.into(),
            preview,
            size_bytes,
            mime_type: mime_type.into(),
            created_ms,
        }
    }

    /// Return a compact summary suitable for embedding into an LLM prompt.
    #[must_use]
    pub fn as_prompt_line(&self) -> String {
        format!(
            "[pointer id={} size={}B mime={}]\n{}{}",
            self.id,
            self.size_bytes,
            self.mime_type,
            self.preview,
            if (self.preview.len() as u64) < self.size_bytes {
                "…"
            } else {
                ""
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_truncates_to_256_bytes_at_char_boundary() {
        // Construct a preview with multi-byte chars spanning past 256.
        let payload: String = "€".repeat(100); // 100 × 3 = 300 bytes
        let p = MemoryPointer::new("abc", &payload, 300, "text/plain", 0);
        assert!(p.preview.len() <= 256);
        assert!(p.preview.is_char_boundary(p.preview.len()));
        // Should contain integer number of Euro signs (each 3 bytes).
        assert!(p.preview.chars().all(|c| c == '€'));
    }

    #[test]
    fn preview_short_kept_verbatim() {
        let p = MemoryPointer::new("abc", "hello", 5, "text/plain", 0);
        assert_eq!(p.preview, "hello");
    }

    #[test]
    fn preview_exact_max_kept() {
        let s = "x".repeat(MemoryPointer::MAX_PREVIEW_BYTES);
        let p = MemoryPointer::new("abc", &s, 256, "text/plain", 0);
        assert_eq!(p.preview.len(), 256);
    }

    #[test]
    fn preview_one_over_max_truncates() {
        let s = "x".repeat(MemoryPointer::MAX_PREVIEW_BYTES + 1);
        let p = MemoryPointer::new("abc", &s, 257, "text/plain", 0);
        assert_eq!(p.preview.len(), 256);
    }

    #[test]
    fn as_prompt_line_shows_id_size_mime_and_preview() {
        let p = MemoryPointer::new("id1", "hi", 2, "text/plain", 0);
        let line = p.as_prompt_line();
        assert!(line.contains("id=id1"));
        assert!(line.contains("size=2B"));
        assert!(line.contains("mime=text/plain"));
        assert!(line.contains("hi"));
        assert!(!line.contains("…")); // size == preview
    }

    #[test]
    fn as_prompt_line_ellipsis_when_truncated() {
        let s = "a".repeat(300);
        let p = MemoryPointer::new("id2", &s, 300, "text/plain", 0);
        let line = p.as_prompt_line();
        assert!(line.contains("…"));
    }

    #[test]
    fn memory_pointer_serde_roundtrip() {
        let p = MemoryPointer::new(
            "abc",
            "preview",
            10_000,
            "application/json",
            1_700_000_000_000,
        );
        let json = serde_json::to_string(&p).unwrap();
        let decoded: MemoryPointer = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, p);
    }
}
