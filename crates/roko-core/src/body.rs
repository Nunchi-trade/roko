//! Engram body — the typed payload carried by a signal.
//!
//! A signal's body can be JSON (structured data), bytes (binary), text (UTF-8),
//! or empty (marker signals). The body's format is explicit — consumers can
//! inspect `body.kind_hint()` before decoding.

use crate::error::{Result, RokoError};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

/// The payload carried by a [`Engram`](crate::Engram).
///
/// Bodies are tagged: consumers can tell at runtime whether the body is
/// structured JSON, raw bytes, text, or absent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "format", content = "data", rename_all = "snake_case")]
pub enum Body {
    /// Empty body — the signal is purely a marker (its kind and tags carry meaning).
    Empty,
    /// UTF-8 text (logs, prompts, messages).
    Text(String),
    /// Structured JSON value.
    Json(serde_json::Value),
    /// Raw bytes (binary artifacts, compressed data).
    Bytes(#[serde(with = "base64_bytes")] Vec<u8>),
}

impl Body {
    /// Create an empty body.
    #[must_use]
    pub const fn empty() -> Self {
        Self::Empty
    }

    /// Create a text body.
    pub fn text(s: impl Into<String>) -> Self {
        Self::Text(s.into())
    }

    /// Serialize any serde-compatible type as the body.
    ///
    /// # Errors
    ///
    /// Returns [`RokoError::BodyEncode`] if serialization fails.
    pub fn from_json<T: Serialize>(value: &T) -> Result<Self> {
        let json = serde_json::to_value(value).map_err(RokoError::body_encode)?;
        Ok(Self::Json(json))
    }

    /// Create a bytes body.
    #[must_use]
    pub fn bytes(b: impl Into<Vec<u8>>) -> Self {
        Self::Bytes(b.into())
    }

    /// Decode a JSON body into a typed value.
    ///
    /// # Errors
    ///
    /// Returns [`RokoError::BodyDecode`] if the body is not JSON, or if the
    /// JSON does not match the requested shape.
    pub fn as_json<T: DeserializeOwned>(&self) -> Result<T> {
        match self {
            Self::Json(v) => serde_json::from_value(v.clone()).map_err(RokoError::body_decode),
            other => Err(RokoError::BodyDecode(format!(
                "expected JSON body, got {}",
                other.kind_hint()
            ))),
        }
    }

    /// Get the text of a text body.
    ///
    /// # Errors
    ///
    /// Returns [`RokoError::BodyDecode`] if the body is not text.
    pub fn as_text(&self) -> Result<&str> {
        match self {
            Self::Text(s) => Ok(s.as_str()),
            other => Err(RokoError::BodyDecode(format!(
                "expected text body, got {}",
                other.kind_hint()
            ))),
        }
    }

    /// Get the bytes of a bytes body.
    ///
    /// # Errors
    ///
    /// Returns [`RokoError::BodyDecode`] if the body is not bytes.
    pub fn as_bytes(&self) -> Result<&[u8]> {
        match self {
            Self::Bytes(b) => Ok(b.as_slice()),
            other => Err(RokoError::BodyDecode(format!(
                "expected bytes body, got {}",
                other.kind_hint()
            ))),
        }
    }

    /// Human-readable hint of the body's format (for error messages).
    #[must_use]
    pub const fn kind_hint(&self) -> &'static str {
        match self {
            Self::Empty => "empty",
            Self::Text(_) => "text",
            Self::Json(_) => "json",
            Self::Bytes(_) => "bytes",
        }
    }

    /// Approximate size of the body's payload in bytes (for budget tracking).
    #[must_use]
    pub fn byte_size(&self) -> usize {
        match self {
            Self::Empty => 0,
            Self::Text(s) => s.len(),
            Self::Json(v) => v.to_string().len(),
            Self::Bytes(b) => b.len(),
        }
    }

    /// Canonical byte encoding of the body (stable for content-hashing).
    ///
    /// Uses a JSON encoding for all body types so content hashes are stable
    /// across serde versions.
    #[must_use]
    pub fn canonical_bytes(&self) -> Vec<u8> {
        // Stable: same content → same bytes → same hash.
        serde_json::to_vec(self).unwrap_or_default()
    }
}

/// Serde support: encode/decode byte bodies as base64 so JSON stays valid UTF-8.
mod base64_bytes {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&encode_base64(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let s = String::deserialize(d)?;
        decode_base64(&s).map_err(serde::de::Error::custom)
    }

    fn encode_base64(bytes: &[u8]) -> String {
        const CHARS: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
        let chunks = bytes.chunks_exact(3);
        let remainder = chunks.remainder();
        for chunk in chunks {
            let n = (u32::from(chunk[0]) << 16) | (u32::from(chunk[1]) << 8) | u32::from(chunk[2]);
            out.push(CHARS[((n >> 18) & 0x3f) as usize] as char);
            out.push(CHARS[((n >> 12) & 0x3f) as usize] as char);
            out.push(CHARS[((n >> 6) & 0x3f) as usize] as char);
            out.push(CHARS[(n & 0x3f) as usize] as char);
        }
        match remainder {
            [a] => {
                let n = u32::from(*a) << 16;
                out.push(CHARS[((n >> 18) & 0x3f) as usize] as char);
                out.push(CHARS[((n >> 12) & 0x3f) as usize] as char);
                out.push('=');
                out.push('=');
            }
            [a, b] => {
                let n = (u32::from(*a) << 16) | (u32::from(*b) << 8);
                out.push(CHARS[((n >> 18) & 0x3f) as usize] as char);
                out.push(CHARS[((n >> 12) & 0x3f) as usize] as char);
                out.push(CHARS[((n >> 6) & 0x3f) as usize] as char);
                out.push('=');
            }
            _ => {}
        }
        out
    }

    fn decode_base64(s: &str) -> Result<Vec<u8>, String> {
        const fn val(b: u8) -> Option<u8> {
            match b {
                b'A'..=b'Z' => Some(b - b'A'),
                b'a'..=b'z' => Some(b - b'a' + 26),
                b'0'..=b'9' => Some(b - b'0' + 52),
                b'+' => Some(62),
                b'/' => Some(63),
                _ => None,
            }
        }
        let bytes = s.as_bytes();
        if bytes.len() % 4 != 0 {
            return Err("base64 length not multiple of 4".into());
        }
        let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
        for chunk in bytes.chunks_exact(4) {
            let mut vals = [0u8; 4];
            let mut pad = 0;
            for (i, &c) in chunk.iter().enumerate() {
                if c == b'=' {
                    pad += 1;
                    vals[i] = 0;
                } else {
                    vals[i] = val(c).ok_or_else(|| format!("invalid base64 byte: {c}"))?;
                }
            }
            let n = (u32::from(vals[0]) << 18)
                | (u32::from(vals[1]) << 12)
                | (u32::from(vals[2]) << 6)
                | u32::from(vals[3]);
            out.push(((n >> 16) & 0xff) as u8);
            if pad < 2 {
                out.push(((n >> 8) & 0xff) as u8);
            }
            if pad < 1 {
                out.push((n & 0xff) as u8);
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_body_has_zero_size() {
        assert_eq!(Body::empty().byte_size(), 0);
    }

    #[test]
    fn text_body_roundtrip() {
        let b = Body::text("hello");
        assert_eq!(b.as_text().unwrap(), "hello");
    }

    #[test]
    fn decoding_wrong_type_errors() {
        let b = Body::text("hi");
        assert!(b.as_bytes().is_err());
        assert!(b.as_json::<serde_json::Value>().is_err());
    }

    #[test]
    fn json_body_typed_decode() {
        #[derive(Serialize, Deserialize, PartialEq, Debug)]
        struct Thing {
            name: String,
            count: u32,
        }
        let orig = Thing {
            name: "x".into(),
            count: 7,
        };
        let body = Body::from_json(&orig).unwrap();
        let decoded: Thing = body.as_json().unwrap();
        assert_eq!(orig, decoded);
    }

    #[test]
    fn bytes_body_base64_roundtrip() {
        let data = vec![0u8, 1, 2, 3, 255, 128, 64];
        let body = Body::bytes(data.clone());
        let json = serde_json::to_string(&body).unwrap();
        let parsed: Body = serde_json::from_str(&json).unwrap();
        assert_eq!(body, parsed);
        assert_eq!(parsed.as_bytes().unwrap(), data.as_slice());
    }

    #[test]
    fn canonical_bytes_stable() {
        let a = Body::text("stable");
        let b = Body::text("stable");
        assert_eq!(a.canonical_bytes(), b.canonical_bytes());
    }

    #[test]
    fn byte_size_reflects_content() {
        assert_eq!(Body::text("12345").byte_size(), 5);
        assert_eq!(Body::bytes(vec![0, 1, 2]).byte_size(), 3);
    }
}
