//! Hyperdimensional Computing (HDC) fingerprints for symbols and files.
//!
//! Each fingerprint is a 10,240-bit binary vector. Similar code produces
//! similar fingerprints, enabling fast similarity search via Hamming distance.
//! Comparison is pure bitwise XOR + popcount and completes well under 1ms.

use roko_core::language::{Symbol, SymbolKind};

use crate::parser::SourceFile;

/// Number of `u64` words in a fingerprint (10,240 / 64 = 160).
const WORDS: usize = 160;
/// Total bits in a fingerprint.
const TOTAL_BITS: usize = WORDS * 64;

// ─── Deterministic PRNG ─────────────────────────────────────────────────

/// Splitmix64 step — a fast, well-distributed bijective function.
const fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Hash bytes into a 64-bit seed using FNV-1a.
fn fnv1a(data: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &byte in data {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    if hash == 0 {
        hash = 0xA5A5_A5A5_5A5A_5A5A;
    }
    hash
}

/// Generate a deterministic 10,240-bit vector from a byte seed.
fn vector_from_seed(seed: &[u8]) -> [u64; WORDS] {
    let mut state = fnv1a(seed);
    let mut bits = [0u64; WORDS];
    for word in &mut bits {
        *word = splitmix64(&mut state);
    }
    bits
}

/// Majority-vote bundle of multiple vectors.
fn bundle(vectors: &[[u64; WORDS]]) -> [u64; WORDS] {
    if vectors.is_empty() {
        return [0u64; WORDS];
    }
    let len = vectors.len();
    let threshold = len / 2;
    let mut result = [0u64; WORDS];
    for (word_idx, slot) in result.iter_mut().enumerate() {
        let mut word = 0u64;
        for bit_idx in 0..64 {
            let mut ones = 0usize;
            for vec in vectors {
                ones += ((vec[word_idx] >> bit_idx) & 1) as usize;
            }
            if ones > threshold {
                word |= 1u64 << bit_idx;
            }
        }
        *slot = word;
    }
    result
}

/// XOR bind two vectors.
fn bind(a: &[u64; WORDS], b: &[u64; WORDS]) -> [u64; WORDS] {
    let mut result = [0u64; WORDS];
    for (slot, (left, right)) in result.iter_mut().zip(a.iter().zip(b.iter())) {
        *slot = left ^ right;
    }
    result
}

/// Count differing bits between two vectors (Hamming distance).
fn hamming_distance(a: &[u64; WORDS], b: &[u64; WORDS]) -> u32 {
    let mut diff = 0u32;
    for (left, right) in a.iter().zip(b.iter()) {
        diff += (left ^ right).count_ones();
    }
    diff
}

// ─── HdcFingerprint ─────────────────────────────────────────────────────

/// A 10,240-bit hyperdimensional computing fingerprint.
///
/// Fingerprints encode the kind, name, and contextual content of code
/// artefacts into a fixed-width binary vector. Similar artefacts produce
/// similar fingerprints.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HdcFingerprint {
    bits: [u64; WORDS],
}

impl HdcFingerprint {
    /// Raw bit words (for inspection / serialisation).
    pub const fn words(&self) -> &[u64; WORDS] {
        &self.bits
    }

    /// Cosine similarity approximated via normalised Hamming distance.
    ///
    /// Returns a value in `[0.0, 1.0]` where 1.0 means identical fingerprints.
    #[allow(clippy::cast_precision_loss)]
    pub fn similarity(&self, other: &Self) -> f64 {
        let dist = hamming_distance(&self.bits, &other.bits);
        1.0 - (f64::from(dist) / TOTAL_BITS as f64)
    }
}

/// Compute the similarity between two fingerprints.
///
/// Convenience wrapper around [`HdcFingerprint::similarity`].
pub fn similarity(a: &HdcFingerprint, b: &HdcFingerprint) -> f64 {
    a.similarity(b)
}

// ─── Fingerprinting functions ───────────────────────────────────────────

/// Deterministic role vector for a [`SymbolKind`].
fn role_vector(kind: &SymbolKind) -> [u64; WORDS] {
    let seed: &[u8] = match kind {
        SymbolKind::Function => b"roko:role:function",
        SymbolKind::Struct => b"roko:role:struct",
        SymbolKind::Enum => b"roko:role:enum",
        SymbolKind::Trait => b"roko:role:trait",
        SymbolKind::Const => b"roko:role:const",
        SymbolKind::Type => b"roko:role:type",
        SymbolKind::Module => b"roko:role:module",
        SymbolKind::Impl => b"roko:role:impl",
        _ => b"roko:role:unknown",
    };
    vector_from_seed(seed)
}

/// Encode a name into an HDC vector via character trigrams.
///
/// Short names (fewer than 3 characters) are seeded directly.
fn encode_name(name: &str) -> [u64; WORDS] {
    let chars: Vec<char> = name.chars().collect();
    if chars.len() < 3 {
        return vector_from_seed(name.as_bytes());
    }

    let trigrams: Vec<[u64; WORDS]> = chars
        .windows(3)
        .map(|w| {
            let trigram: String = w.iter().collect();
            vector_from_seed(trigram.as_bytes())
        })
        .collect();

    bundle(&trigrams)
}

/// Generate an HDC fingerprint for a single [`Symbol`].
///
/// The fingerprint combines:
/// 1. A role vector derived from the symbol kind.
/// 2. A name vector derived from character trigrams.
/// 3. A context vector seeded from the supplied context bytes.
///
/// Formula: `bind(role, bundle(name, context))`
pub fn fingerprint_symbol(symbol: &Symbol, context: &[u8]) -> HdcFingerprint {
    let role_vec = role_vector(&symbol.kind);
    let name_vec = encode_name(&symbol.name);
    let ctx_vec = vector_from_seed(context);
    let combined = bundle(&[name_vec, ctx_vec]);
    HdcFingerprint {
        bits: bind(&role_vec, &combined),
    }
}

/// Generate an HDC fingerprint for an entire source file.
///
/// Bundles fingerprints of all symbols in the file. If the file has no
/// symbols, the fingerprint is seeded from the raw content.
pub fn fingerprint_file(source: &SourceFile) -> HdcFingerprint {
    if source.symbols.is_empty() {
        return HdcFingerprint {
            bits: vector_from_seed(source.content.as_bytes()),
        };
    }

    let sym_fps: Vec<[u64; WORDS]> = source
        .symbols
        .iter()
        .map(|sym| {
            let fp = fingerprint_symbol(sym, source.content.as_bytes());
            fp.bits
        })
        .collect();

    HdcFingerprint {
        bits: bundle(&sym_fps),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::language::Visibility;
    use std::time::Instant;

    fn make_sym(name: &str, kind: SymbolKind) -> Symbol {
        Symbol {
            name: name.into(),
            kind,
            visibility: Visibility::Public,
            line: 1,
        }
    }

    #[test]
    fn identical_symbols_identical_fingerprints() {
        let sym = make_sym("process", SymbolKind::Function);
        let fp1 = fingerprint_symbol(&sym, b"fn process() {}");
        let fp2 = fingerprint_symbol(&sym, b"fn process() {}");
        let sim = fp1.similarity(&fp2);
        assert!(
            (sim - 1.0).abs() < 1e-9,
            "Same symbol+context should produce identical fingerprints, got {sim}"
        );
    }

    #[test]
    fn similar_names_high_similarity() {
        let sym1 = make_sym("process_input", SymbolKind::Function);
        let sym2 = make_sym("process_output", SymbolKind::Function);
        let ctx = b"fn process(data: &str) -> Result<String>";
        let fp1 = fingerprint_symbol(&sym1, ctx);
        let fp2 = fingerprint_symbol(&sym2, ctx);
        let sim = fp1.similarity(&fp2);
        assert!(
            sim > 0.5,
            "Similar function names with same context should be similar, got {sim}"
        );
    }

    #[test]
    fn different_kinds_lower_similarity() {
        let func = make_sym("Config", SymbolKind::Function);
        let strct = make_sym("Config", SymbolKind::Struct);
        let ctx = b"Config";
        let fp_func = fingerprint_symbol(&func, ctx);
        let fp_struct = fingerprint_symbol(&strct, ctx);
        let sim = fp_func.similarity(&fp_struct);
        // Same name but different kinds — should not be near-identical.
        assert!(
            sim < 0.9,
            "Different kinds with same name should have reduced similarity, got {sim}"
        );
    }

    #[test]
    fn completely_different_symbols_low_similarity() {
        let sym1 = make_sym("parse_config", SymbolKind::Function);
        let sym2 = make_sym("Color", SymbolKind::Enum);
        let fp1 = fingerprint_symbol(&sym1, b"fn parse_config(path: &Path)");
        let fp2 = fingerprint_symbol(&sym2, b"enum Color { Red, Green }");
        let sim = fp1.similarity(&fp2);
        assert!(
            sim < 0.7,
            "Completely different symbols should have low similarity, got {sim}"
        );
    }

    #[test]
    fn fingerprint_file_deterministic() {
        let file = SourceFile {
            path: "test.rs".into(),
            language: "rust".into(),
            content: "fn hello() {}\nfn world() {}".into(),
            symbols: vec![
                make_sym("hello", SymbolKind::Function),
                make_sym("world", SymbolKind::Function),
            ],
            imports: vec![],
        };
        let fp1 = fingerprint_file(&file);
        let fp2 = fingerprint_file(&file);
        assert_eq!(fp1, fp2);
    }

    #[test]
    fn fingerprint_file_empty_symbols() {
        let file = SourceFile {
            path: "empty.txt".into(),
            language: "text".into(),
            content: "just some text".into(),
            symbols: vec![],
            imports: vec![],
        };
        let fp = fingerprint_file(&file);
        // Should not panic; fingerprint is seeded from content.
        assert_ne!(fp.bits, [0u64; WORDS]);
    }

    #[test]
    fn similarity_function_is_consistent() {
        let sym = make_sym("test", SymbolKind::Function);
        let fp = fingerprint_symbol(&sym, b"context");
        let sim_method = fp.similarity(&fp);
        let sim_fn = similarity(&fp, &fp);
        assert!((sim_method - sim_fn).abs() < 1e-9);
    }

    #[test]
    fn comparison_performance_under_1ms() {
        let sym1 = make_sym("alpha", SymbolKind::Function);
        let sym2 = make_sym("beta", SymbolKind::Struct);
        let fp1 = fingerprint_symbol(&sym1, b"alpha context");
        let fp2 = fingerprint_symbol(&sym2, b"beta context");

        let start = Instant::now();
        for _ in 0..10_000 {
            let _ = fp1.similarity(&fp2);
        }
        let elapsed = start.elapsed();
        let per_op = elapsed / 10_000;
        // Each comparison must be < 1ms; should be well under 1us.
        assert!(
            per_op.as_micros() < 1_000,
            "Single comparison took {per_op:?}, expected < 1ms"
        );
    }

    #[test]
    fn self_similarity_is_one() {
        let fp = HdcFingerprint {
            bits: vector_from_seed(b"anything"),
        };
        let sim = fp.similarity(&fp);
        assert!(
            (sim - 1.0).abs() < 1e-9,
            "Self-similarity should be 1.0, got {sim}"
        );
    }

    #[test]
    fn short_name_encoding() {
        // Names shorter than 3 chars should not panic and produce valid vectors.
        let fp = fingerprint_symbol(&make_sym("x", SymbolKind::Const), b"");
        assert_ne!(fp.bits, [0u64; WORDS]);
    }
}
