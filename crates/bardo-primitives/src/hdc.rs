//! 10,240-bit hyperdimensional computing (HDC) vector.

use core::convert::TryFrom;
use uuid::Uuid;

const fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// 10,240-bit binary sparse distributed vector.
///
/// Three core operations: XOR bind, majority-vote bundle, Hamming similarity.
/// All operations are CPU-cache-friendly bit manipulation — no floating point,
/// no matrix multiply, no GPU required.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(
    feature = "rkyv",
    derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)
)]
pub struct HdcVector {
    bits: [u64; 160],
}

impl serde::Serialize for HdcVector {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // Serialize as a byte slice (1280 bytes LE-packed u64 words).
        serializer.serialize_bytes(&self.to_bytes())
    }
}

impl<'de> serde::Deserialize<'de> for HdcVector {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct BytesVisitor;
        impl<'de> serde::de::Visitor<'de> for BytesVisitor {
            type Value = [u8; 1280];

            fn expecting(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
                write!(f, "1280 bytes (10240-bit HDC vector)")
            }

            fn visit_bytes<E: serde::de::Error>(self, v: &[u8]) -> Result<Self::Value, E> {
                if v.len() != 1280 {
                    return Err(E::invalid_length(v.len(), &self));
                }
                let mut out = [0u8; 1280];
                out.copy_from_slice(v);
                Ok(out)
            }

            fn visit_byte_buf<E: serde::de::Error>(self, v: Vec<u8>) -> Result<Self::Value, E> {
                self.visit_bytes(&v)
            }

            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<Self::Value, A::Error> {
                let mut out = [0u8; 1280];
                for (i, slot) in out.iter_mut().enumerate() {
                    match seq.next_element::<u8>()? {
                        Some(b) => *slot = b,
                        None => return Err(serde::de::Error::invalid_length(i, &self)),
                    }
                }
                Ok(out)
            }
        }
        let bytes = deserializer.deserialize_bytes(BytesVisitor)?;
        Ok(Self::from_bytes(&bytes))
    }
}

impl HdcVector {
    /// Returns an all-zero vector.
    #[must_use]
    pub const fn zeros() -> Self {
        Self { bits: [0; 160] }
    }

    /// Returns a pseudo-random vector seeded from a random UUID.
    #[must_use]
    pub fn random() -> Self {
        let seed = Uuid::new_v4().as_u128();
        let seed_bytes = seed.to_le_bytes();
        let mut low_bytes = [0u8; 8];
        low_bytes.copy_from_slice(&seed_bytes[..8]);
        let mut high_bytes = [0u8; 8];
        high_bytes.copy_from_slice(&seed_bytes[8..]);
        let mut state = u64::from_le_bytes(low_bytes) ^ u64::from_le_bytes(high_bytes);
        if state == 0 {
            state = 0xA5A5_A5A5_5A5A_5A5A;
        }

        let mut bits = [0u64; 160];
        for word in &mut bits {
            *word = splitmix64(&mut state);
        }
        Self { bits }
    }

    /// Binds two vectors using XOR. Involution: `bind(bind(a, b), b) == a`.
    #[must_use]
    pub fn bind(&self, other: &Self) -> Self {
        let mut bits = [0u64; 160];
        for (slot, (left, right)) in bits.iter_mut().zip(self.bits.iter().zip(other.bits.iter())) {
            *slot = left ^ right;
        }
        Self { bits }
    }

    /// Bundles a slice of vectors using majority vote (tie → 0).
    #[must_use]
    pub fn bundle(vectors: &[&Self]) -> Self {
        if vectors.is_empty() {
            return Self::zeros();
        }

        let len = vectors.len();
        let mut bits = [0u64; 160];
        for (word_index, slot) in bits.iter_mut().enumerate() {
            let mut word = 0u64;
            for bit_index in 0..64 {
                let mut ones = 0usize;
                for vector in vectors {
                    ones += ((vector.bits[word_index] >> bit_index) & 1) as usize;
                }
                if ones * 2 > len {
                    word |= 1u64 << bit_index;
                }
            }
            *slot = word;
        }
        Self { bits }
    }

    /// Rotates bits left by `n` positions (cyclic permutation for sequence encoding).
    #[must_use]
    pub fn permute(&self, n: usize) -> Self {
        let bits_len = self.bits.len() * 64;
        let n = n % bits_len;
        if n == 0 {
            return *self;
        }

        let word_shift = n / 64;
        let bit_shift = n % 64;
        let mut bits = [0u64; 160];

        for (index, slot) in bits.iter_mut().enumerate() {
            let src0 = (index + 160 - word_shift) % 160;
            *slot = if bit_shift == 0 {
                self.bits[src0]
            } else {
                let src1 = (src0 + 159) % 160;
                (self.bits[src0] << bit_shift) | (self.bits[src1] >> (64 - bit_shift))
            };
        }

        Self { bits }
    }

    /// Serialize the vector to 1280 little-endian bytes.
    #[must_use]
    pub fn to_bytes(&self) -> [u8; 1280] {
        let mut out = [0u8; 1280];
        for (i, word) in self.bits.iter().enumerate() {
            out[i * 8..(i + 1) * 8].copy_from_slice(&word.to_le_bytes());
        }
        out
    }

    /// Deserialize a vector from 1280 little-endian bytes.
    #[must_use]
    pub fn from_bytes(bytes: &[u8; 1280]) -> Self {
        let mut bits = [0u64; 160];
        for (i, word) in bits.iter_mut().enumerate() {
            let mut buf = [0u8; 8];
            buf.copy_from_slice(&bytes[i * 8..(i + 1) * 8]);
            *word = u64::from_le_bytes(buf);
        }
        Self { bits }
    }

    /// Create a deterministic vector from a byte seed.
    ///
    /// Uses FNV-1a to hash the seed into a 64-bit state, then splitmix64 to fill bits.
    /// Identical seeds always produce identical vectors. Useful for stable role vectors.
    #[must_use]
    pub fn from_seed(seed: &[u8]) -> Self {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325; // FNV-1a offset basis
        for &byte in seed {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3); // FNV prime
        }
        if hash == 0 {
            hash = 0xA5A5_A5A5_5A5A_5A5A;
        }

        let mut bits = [0u64; 160];
        for word in &mut bits {
            *word = splitmix64(&mut hash);
        }
        Self { bits }
    }

    /// Returns the Hamming similarity in the range `[0, 1]`.
    pub fn similarity(&self, other: &Self) -> f32 {
        let mut differing_bits = 0u32;
        for (left, right) in self.bits.iter().zip(other.bits.iter()) {
            differing_bits += (left ^ right).count_ones();
        }
        let differing_bits = u16::try_from(differing_bits).unwrap_or(u16::MAX);
        1.0_f32 - (f32::from(differing_bits) / 10_240.0_f32)
    }

    /// Returns Hamming similarity against an rkyv-archived vector (zero-copy).
    ///
    /// On little-endian platforms, the archived representation of `[u64; 160]`
    /// is identical to the in-memory layout, so this reads directly from the
    /// mmap'd buffer with no deserialization.
    #[cfg(feature = "rkyv")]
    pub fn similarity_archived(&self, archived: &ArchivedHdcVector) -> f32 {
        let mut differing_bits = 0u32;
        for (left, right) in self.bits.iter().zip(archived.bits.iter()) {
            let right_u64: u64 = (*right).into();
            differing_bits += (left ^ right_u64).count_ones();
        }
        let differing_bits = u16::try_from(differing_bits).unwrap_or(u16::MAX);
        1.0_f32 - (f32::from(differing_bits) / 10_240.0_f32)
    }
}

/// Compute a deterministic HDC fingerprint for any serializable value.
///
/// The value is first encoded with `serde_json`, then mapped into a
/// 10,240-bit vector using the crate's deterministic seed expansion.
#[must_use]
pub fn fingerprint(value: &impl serde::Serialize) -> HdcVector {
    let seed = serde_json::to_vec(value).unwrap_or_default();
    HdcVector::from_seed(&seed)
}

/// Compute a deterministic HDC fingerprint for raw text.
///
/// This is a convenience wrapper for callers that already have a
/// canonical text blob and do not need to serialize a structured value
/// through `serde_json` first.
#[must_use]
pub fn text_fingerprint(text: &str) -> HdcVector {
    HdcVector::from_seed(text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::{HdcVector, fingerprint, text_fingerprint};

    #[test]
    fn hdc_bind_involution() {
        let a = HdcVector::random();
        let b = HdcVector::random();
        let recovered = a.bind(&b).bind(&b);
        assert!((recovered.similarity(&a) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn hdc_similarity_self() {
        let vector = HdcVector::random();
        assert!((vector.similarity(&vector) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn hdc_bundle_tie_rule() {
        let mut a = HdcVector::zeros();
        let mut b = HdcVector::zeros();
        a.bits[0] = 1;
        b.bits[0] = 0;
        let bundled = HdcVector::bundle(&[&a, &b]);
        assert_eq!(bundled.bits[0], 0);
    }

    #[test]
    fn hdc_bytes_roundtrip() {
        let v = HdcVector::random();
        let bytes = v.to_bytes();
        let recovered = HdcVector::from_bytes(&bytes);
        assert_eq!(v, recovered);
    }

    #[test]
    fn hdc_from_seed_deterministic() {
        let a = HdcVector::from_seed(b"function");
        let b = HdcVector::from_seed(b"function");
        assert_eq!(a, b);
    }

    #[test]
    fn hdc_from_seed_distinct() {
        let a = HdcVector::from_seed(b"function");
        let b = HdcVector::from_seed(b"struct");
        assert!(a.similarity(&b) < 0.6);
    }

    #[test]
    fn hdc_serde_roundtrip_json() {
        let v = HdcVector::from_seed(b"serde roundtrip");
        let json = serde_json::to_string(&v).expect("serialize");
        let decoded: HdcVector = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(v, decoded);
    }

    #[test]
    fn hdc_serde_inside_struct() {
        #[derive(serde::Serialize, serde::Deserialize)]
        struct Wrapper {
            label: String,
            vector: HdcVector,
        }
        let w = Wrapper {
            label: "t".into(),
            vector: HdcVector::from_seed(b"inside struct"),
        };
        let json = serde_json::to_string(&w).expect("serialize");
        let decoded: Wrapper = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded.label, "t");
        assert_eq!(decoded.vector, w.vector);
    }

    #[test]
    fn hdc_fingerprint_is_deterministic() {
        let left = fingerprint(&serde_json::json!({"a": 1, "b": [2, 3]}));
        let right = fingerprint(&serde_json::json!({"a": 1, "b": [2, 3]}));
        assert_eq!(left, right);
    }

    #[test]
    fn hdc_text_fingerprint_is_deterministic() {
        let left = text_fingerprint("trigger_kind=webhook_dispatch\nagent_template=a");
        let right = text_fingerprint("trigger_kind=webhook_dispatch\nagent_template=a");
        assert_eq!(left, right);
    }
}
