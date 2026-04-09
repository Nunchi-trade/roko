//! Content-addressed hashing for signals.
//!
//! Signals are identified by a [`ContentHash`] — a BLAKE3 digest of their
//! canonical encoding. This gives us:
//!
//! - **Deduplication**: identical signals collapse to one id
//! - **Integrity**: a signal's id proves its content
//! - **Provenance**: lineage chains are hash-linked (tamper-evident)
//! - **Addressable storage**: substrates can be indexed by hash
//!
//! We use BLAKE3 (not SHA-256) because it's faster and gives us streaming hashing
//! for free, which matters once signals include file contents or large payloads.

use serde::{Deserialize, Serialize};
use std::fmt;

/// A 32-byte content-addressed identifier (BLAKE3 digest).
///
/// Two signals with identical canonical encoding share the same `ContentHash`.
/// The hash is computed over the signal's body and its identity fields, but
/// **not** its score or decay — those can change without changing identity.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ContentHash(#[serde(with = "hex_bytes")] pub [u8; 32]);

impl ContentHash {
    /// Compute a content hash from arbitrary bytes.
    #[must_use]
    pub fn of(bytes: &[u8]) -> Self {
        Self(*blake3::hash(bytes).as_bytes())
    }

    /// Hex-encoded representation (64 chars).
    #[must_use]
    pub fn to_hex(&self) -> String {
        use std::fmt::Write;
        let mut s = String::with_capacity(64);
        for byte in self.0 {
            let _ = write!(s, "{byte:02x}");
        }
        s
    }

    /// Short form for logs/display (first 8 hex chars).
    #[must_use]
    pub fn short(&self) -> String {
        format!(
            "{:02x}{:02x}{:02x}{:02x}",
            self.0[0], self.0[1], self.0[2], self.0[3]
        )
    }

    /// Parse a hex string into a `ContentHash`. Returns `None` for malformed input.
    #[must_use]
    pub fn from_hex(s: &str) -> Option<Self> {
        if s.len() != 64 {
            return None;
        }
        let mut bytes = [0u8; 32];
        for (i, chunk) in s.as_bytes().chunks_exact(2).enumerate() {
            let hi = hex_digit(chunk[0])?;
            let lo = hex_digit(chunk[1])?;
            bytes[i] = (hi << 4) | lo;
        }
        Some(Self(bytes))
    }
}

impl fmt::Debug for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ContentHash({})", self.short())
    }
}

impl fmt::Display for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.short())
    }
}

const fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Serde support: render `ContentHash` as a hex string rather than a byte array.
mod hex_bytes {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8; 32], s: S) -> Result<S::Ok, S::Error> {
        use std::fmt::Write;
        let mut hex = String::with_capacity(64);
        for byte in bytes {
            let _ = write!(hex, "{byte:02x}");
        }
        s.serialize_str(&hex)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 32], D::Error> {
        let s = String::deserialize(d)?;
        super::ContentHash::from_hex(&s)
            .map(|h| h.0)
            .ok_or_else(|| serde::de::Error::custom("invalid ContentHash hex"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_deterministic() {
        let a = ContentHash::of(b"hello world");
        let b = ContentHash::of(b"hello world");
        assert_eq!(a, b);
    }

    #[test]
    fn hash_distinguishes_content() {
        let a = ContentHash::of(b"hello");
        let b = ContentHash::of(b"world");
        assert_ne!(a, b);
    }

    #[test]
    fn hex_roundtrip() {
        let original = ContentHash::of(b"test data");
        let hex = original.to_hex();
        let parsed = ContentHash::from_hex(&hex).unwrap();
        assert_eq!(original, parsed);
    }

    #[test]
    fn short_form_has_8_chars() {
        let h = ContentHash::of(b"anything");
        assert_eq!(h.short().len(), 8);
    }

    #[test]
    fn invalid_hex_returns_none() {
        assert!(ContentHash::from_hex("too_short").is_none());
        assert!(ContentHash::from_hex(&"g".repeat(64)).is_none());
    }

    #[test]
    fn serde_roundtrip() {
        let h = ContentHash::of(b"roundtrip");
        let json = serde_json::to_string(&h).unwrap();
        let parsed: ContentHash = serde_json::from_str(&json).unwrap();
        assert_eq!(h, parsed);
        // Should be a hex string, not a byte array
        assert!(json.starts_with('"'));
        assert_eq!(json.len(), 66); // "<64 hex>"
    }
}
