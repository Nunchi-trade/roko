//! Helpers for computing and storing episode HDC fingerprints.

use serde::Serialize;

use roko_primitives::hdc::{HdcVector, fingerprint};

/// Canonical prompt/outcome payload used to derive an episode fingerprint.
#[derive(Debug, Serialize)]
struct EpisodeFingerprintInput<'a> {
    prompt: &'a str,
    outcome: &'a str,
}

/// Compute a deterministic HDC fingerprint from an episode prompt/outcome pair.
#[must_use]
pub fn fingerprint_episode(prompt: &str, outcome: &str) -> HdcVector {
    fingerprint(&EpisodeFingerprintInput { prompt, outcome })
}

/// Encode an HDC fingerprint as standard base64 with padding.
#[must_use]
pub fn encode(vector: &HdcVector) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let bytes = vector.to_bytes();
    let mut encoded = String::with_capacity(1708);

    for chunk in bytes.chunks(3) {
        let b0 = chunk[0];
        let b1 = *chunk.get(1).unwrap_or(&0);
        let b2 = *chunk.get(2).unwrap_or(&0);

        encoded.push(ALPHABET[(b0 >> 2) as usize] as char);
        encoded.push(ALPHABET[((b0 & 0b0000_0011) << 4 | (b1 >> 4)) as usize] as char);

        if chunk.len() > 1 {
            encoded.push(ALPHABET[((b1 & 0b0000_1111) << 2 | (b2 >> 6)) as usize] as char);
        } else {
            encoded.push('=');
        }

        if chunk.len() > 2 {
            encoded.push(ALPHABET[(b2 & 0b0011_1111) as usize] as char);
        } else {
            encoded.push('=');
        }
    }

    encoded
}

/// Decode a standard padded base64-encoded HDC fingerprint.
pub fn decode(encoded: &str) -> Result<HdcVector, String> {
    let cleaned: Vec<u8> = encoded
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect();
    if cleaned.len() % 4 != 0 {
        return Err("base64 fingerprint length must be a multiple of 4".to_string());
    }

    let mut bytes = Vec::with_capacity((cleaned.len() / 4) * 3);
    for chunk in cleaned.chunks(4) {
        let pad = chunk.iter().rev().take_while(|byte| **byte == b'=').count();
        if pad > 2 {
            return Err("base64 fingerprint has invalid padding".to_string());
        }

        let a = decode_sextet(chunk[0])?;
        let b = decode_sextet(chunk[1])?;
        let c = if chunk[2] == b'=' {
            0
        } else {
            decode_sextet(chunk[2])?
        };
        let d = if chunk[3] == b'=' {
            0
        } else {
            decode_sextet(chunk[3])?
        };

        if pad == 1 && chunk[2] == b'=' {
            return Err("base64 fingerprint has invalid single-byte padding".to_string());
        }
        if pad == 2 && (chunk[2] != b'=' || chunk[3] != b'=') {
            return Err("base64 fingerprint has invalid double-byte padding".to_string());
        }
        if pad == 0 && (chunk.contains(&b'=') || chunk.len() != 4) {
            return Err("base64 fingerprint has unexpected padding".to_string());
        }

        bytes.push((a << 2) | (b >> 4));
        if chunk[2] != b'=' {
            bytes.push((b << 4) | (c >> 2));
        }
        if chunk[3] != b'=' {
            bytes.push((c << 6) | d);
        }
    }

    let bytes: [u8; 1280] = bytes
        .try_into()
        .map_err(|_| "decoded fingerprint must be exactly 1280 bytes".to_string())?;
    Ok(HdcVector::from_bytes(&bytes))
}

fn decode_sextet(byte: u8) -> Result<u8, String> {
    match byte {
        b'A'..=b'Z' => Ok(byte - b'A'),
        b'a'..=b'z' => Ok(byte - b'a' + 26),
        b'0'..=b'9' => Ok(byte - b'0' + 52),
        b'+' => Ok(62),
        b'/' => Ok(63),
        b'=' => Err("base64 padding byte found in unexpected position".to_string()),
        _ => Err(format!("invalid base64 byte: {byte}")),
    }
}

#[cfg(test)]
mod tests {
    use super::{decode, encode, fingerprint_episode};

    #[test]
    fn fingerprint_roundtrips_through_base64() {
        let vector = fingerprint_episode("hello prompt", "world outcome");
        let encoded = encode(&vector);
        let decoded = decode(&encoded).expect("decode");
        assert_eq!(vector, decoded);
    }
}
