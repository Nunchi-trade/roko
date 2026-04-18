//! 10,240-bit hyperdimensional computing (HDC) vector.

use core::convert::TryFrom;
use std::collections::HashMap;
use uuid::Uuid;

/// Number of bits in one [`HdcVector`].
pub const HDC_BITS: usize = 10_240;
/// Number of serialized bytes in one [`HdcVector`].
pub const HDC_BYTES: usize = 1_280;

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

/// Incremental majority-vote accumulator for HDC bundling.
///
/// Each added vector contributes `+1` for set bits and `-1` for unset bits.
/// [`BundleAccumulator::finish`] thresholds the vote tally at zero to produce a
/// bundled [`HdcVector`].
#[derive(Debug, Clone)]
pub struct BundleAccumulator {
    votes: Vec<i32>,
    /// Number of vectors added so far.
    pub count: usize,
}

impl Default for BundleAccumulator {
    fn default() -> Self {
        Self::new()
    }
}

impl BundleAccumulator {
    /// Create an empty accumulator.
    #[must_use]
    pub fn new() -> Self {
        Self {
            votes: vec![0; HDC_BITS],
            count: 0,
        }
    }

    /// Add one vector to the running vote tally.
    pub fn add(&mut self, hv: &HdcVector) {
        self.count = self.count.saturating_add(1);
        update_votes_i32(&mut self.votes, hv, 1);
    }

    /// Add one vector with integer weight.
    ///
    /// Negative weights subtract the vector's contribution.
    pub fn add_weighted(&mut self, hv: &HdcVector, weight: i32) {
        self.count = self
            .count
            .saturating_add(usize::try_from(weight.unsigned_abs()).unwrap_or(usize::MAX));
        update_votes_i32(&mut self.votes, hv, weight);
    }

    /// Apply multiplicative decay to the vote tally.
    ///
    /// # Panics
    ///
    /// Panics if `factor` is negative.
    #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
    pub fn decay(&mut self, factor: f32) {
        assert!(factor >= 0.0, "decay factor must be non-negative");
        for vote in &mut self.votes {
            *vote = (*vote as f32 * factor) as i32;
        }
    }

    /// Collapse the vote tally into a bundled [`HdcVector`].
    #[must_use]
    pub fn finish(&self) -> HdcVector {
        let mut bits = [0u64; 160];
        for (word_index, slot) in bits.iter_mut().enumerate() {
            let mut word = 0u64;
            for bit_index in 0..64 {
                let position = word_index * 64 + bit_index;
                if self.votes[position] > 0 {
                    word |= 1u64 << bit_index;
                }
            }
            *slot = word;
        }
        HdcVector { bits }
    }
}

/// Bundle accumulator with automatic temporal decay.
///
/// Each call to [`DecayingBundleAccumulator::add`] decays prior votes before
/// applying the new vector, which biases the finished bundle toward more
/// recent additions.
#[derive(Debug, Clone)]
pub struct DecayingBundleAccumulator {
    votes: Vec<f32>,
    /// Number of vectors added so far.
    pub count: usize,
    decay_factor: f32,
}

impl DecayingBundleAccumulator {
    /// Create a new decaying accumulator.
    ///
    /// # Panics
    ///
    /// Panics if `decay_factor` is outside `(0.0, 1.0]`.
    #[must_use]
    pub fn new(decay_factor: f32) -> Self {
        assert!(
            decay_factor > 0.0 && decay_factor <= 1.0,
            "decay_factor must be in (0.0, 1.0], got {decay_factor}"
        );
        Self {
            votes: vec![0.0; HDC_BITS],
            count: 0,
            decay_factor,
        }
    }

    /// Add one vector after decaying prior votes.
    pub fn add(&mut self, hv: &HdcVector) {
        self.count = self.count.saturating_add(1);
        for vote in &mut self.votes {
            *vote *= self.decay_factor;
        }
        update_votes_f32(&mut self.votes, hv, 1.0);
    }

    /// Return the configured decay factor.
    #[must_use]
    pub const fn decay_factor(&self) -> f32 {
        self.decay_factor
    }

    /// Effective half-life in number of additions.
    #[must_use]
    pub fn half_life(&self) -> f32 {
        -(2.0_f32.ln()) / self.decay_factor.ln()
    }

    /// Collapse the vote tally into a bundled [`HdcVector`].
    #[must_use]
    pub fn finish(&self) -> HdcVector {
        let mut bits = [0u64; 160];
        for (word_index, slot) in bits.iter_mut().enumerate() {
            let mut word = 0u64;
            for bit_index in 0..64 {
                let position = word_index * 64 + bit_index;
                if self.votes[position] > 0.0 {
                    word |= 1u64 << bit_index;
                }
            }
            *slot = word;
        }
        HdcVector { bits }
    }
}

/// Named HDC codebook with brute-force nearest-neighbor lookup.
#[derive(Debug, Clone, Default)]
pub struct ItemMemory {
    entries: HashMap<String, HdcVector>,
}

impl ItemMemory {
    /// Create an empty codebook.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a named concept and its vector.
    pub fn insert(&mut self, name: impl Into<String>, hv: HdcVector) -> Option<HdcVector> {
        self.entries.insert(name.into(), hv)
    }

    /// Insert a deterministic seed-based vector for `name`.
    pub fn insert_seeded(&mut self, name: &str) -> Option<HdcVector> {
        self.insert(name, HdcVector::from_seed(name.as_bytes()))
    }

    /// Look up a concept by exact name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&HdcVector> {
        self.entries.get(name)
    }

    /// Find the `k` nearest named concepts to `query`.
    #[must_use]
    pub fn top_k(&self, query: &HdcVector, k: usize) -> Vec<(&str, f32)> {
        if k == 0 || self.entries.is_empty() {
            return Vec::new();
        }

        let mut scored = self
            .entries
            .iter()
            .map(|(name, hv)| (name.as_str(), query.similarity(hv)))
            .collect::<Vec<_>>();
        scored.sort_by(|left, right| {
            right
                .1
                .partial_cmp(&left.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left.0.cmp(right.0))
        });
        scored.truncate(k);
        scored
    }

    /// Return the nearest named concept to `query`.
    #[must_use]
    pub fn nearest(&self, query: &HdcVector) -> Option<(&str, f32)> {
        self.top_k(query, 1).into_iter().next()
    }

    /// Number of concepts stored in the codebook.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the codebook is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

fn update_votes_i32(votes: &mut [i32], hv: &HdcVector, weight: i32) {
    if weight == 0 {
        return;
    }

    for (word_index, word) in hv.bits.iter().enumerate() {
        for bit_index in 0..64 {
            let position = word_index * 64 + bit_index;
            if (word >> bit_index) & 1 == 1 {
                votes[position] += weight;
            } else {
                votes[position] -= weight;
            }
        }
    }
}

fn update_votes_f32(votes: &mut [f32], hv: &HdcVector, weight: f32) {
    if weight == 0.0 {
        return;
    }

    for (word_index, word) in hv.bits.iter().enumerate() {
        for bit_index in 0..64 {
            let position = word_index * 64 + bit_index;
            if (word >> bit_index) & 1 == 1 {
                votes[position] += weight;
            } else {
                votes[position] -= weight;
            }
        }
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
    use super::{
        BundleAccumulator, DecayingBundleAccumulator, HDC_BITS, HDC_BYTES, HdcVector, ItemMemory,
        fingerprint, text_fingerprint,
    };

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

    #[test]
    fn hdc_constants_match_shape() {
        assert_eq!(HDC_BITS, 10_240);
        assert_eq!(HDC_BYTES, 1_280);
    }

    #[test]
    fn bundle_accumulator_empty_finishes_to_zero() {
        let accumulator = BundleAccumulator::new();
        assert_eq!(accumulator.finish(), HdcVector::zeros());
    }

    #[test]
    fn bundle_accumulator_single_vector_round_trips() {
        let vector = HdcVector::from_seed(b"bundle-single");
        let mut accumulator = BundleAccumulator::new();
        accumulator.add(&vector);
        assert_eq!(accumulator.finish(), vector);
    }

    #[test]
    fn bundle_accumulator_weight_matches_repeated_adds() {
        let a = HdcVector::from_seed(b"bundle-a");
        let b = HdcVector::from_seed(b"bundle-b");

        let mut repeated = BundleAccumulator::new();
        repeated.add(&a);
        repeated.add(&a);
        repeated.add(&a);
        repeated.add(&b);
        repeated.add(&b);

        let mut weighted = BundleAccumulator::new();
        weighted.add_weighted(&a, 3);
        weighted.add_weighted(&b, 2);

        assert_eq!(weighted.finish(), repeated.finish());
    }

    #[test]
    fn bundle_accumulator_decay_extremes_behave() {
        let vector = HdcVector::from_seed(b"bundle-decay");
        let mut accumulator = BundleAccumulator::new();
        accumulator.add(&vector);
        let original = accumulator.finish();

        accumulator.decay(1.0);
        assert_eq!(accumulator.finish(), original);

        accumulator.decay(0.0);
        assert_eq!(accumulator.finish(), HdcVector::zeros());
    }

    #[test]
    fn item_memory_seeded_insert_matches_from_seed() {
        let mut memory = ItemMemory::new();
        memory.insert_seeded("rust");
        assert_eq!(memory.get("rust"), Some(&HdcVector::from_seed(b"rust")));
    }

    #[test]
    fn item_memory_nearest_returns_exact_match() {
        let rust = HdcVector::from_seed(b"rust");
        let mut memory = ItemMemory::new();
        memory.insert("rust", rust);
        memory.insert_seeded("python");

        let nearest = memory.nearest(&rust).expect("nearest match");
        assert_eq!(nearest.0, "rust");
        assert!((nearest.1 - 1.0).abs() < 1e-6);
    }

    #[test]
    fn item_memory_top_k_limits_results() {
        let mut memory = ItemMemory::new();
        for name in ["rust", "python", "go", "zig"] {
            memory.insert_seeded(name);
        }

        let results = memory.top_k(&HdcVector::from_seed(b"rust"), 3);
        assert_eq!(results.len(), 3);
        assert!(results.windows(2).all(|window| window[0].1 >= window[1].1));
    }

    #[test]
    fn decaying_bundle_accumulator_tracks_recent_vectors() {
        let a = HdcVector::from_seed(b"decay-a");
        let b = HdcVector::from_seed(b"decay-b");
        let mut accumulator = DecayingBundleAccumulator::new(0.0001);
        accumulator.add(&a);
        accumulator.add(&b);

        let bundled = accumulator.finish();
        assert!(bundled.similarity(&b) > bundled.similarity(&a));
    }

    #[test]
    fn decaying_bundle_half_life_matches_docs() {
        let accumulator = DecayingBundleAccumulator::new(0.95);
        assert!((accumulator.half_life() - 13.5).abs() < 0.5);
    }
}
