//! Property-based tests for HDC vectors.

use proptest::prelude::*;
use roko_primitives::HdcVector;

fn arb_hdc_vector() -> impl Strategy<Value = HdcVector> {
    any::<[u8; 32]>().prop_map(|seed| HdcVector::from_seed(&seed))
}

proptest! {
    /// bind is its own inverse: bind(bind(a, b), b) == a
    #[test]
    fn bind_is_involution(a in arb_hdc_vector(), b in arb_hdc_vector()) {
        let recovered = a.bind(&b).bind(&b);
        prop_assert!((recovered.similarity(&a) - 1.0).abs() < 1e-6,
            "bind(bind(a,b),b) should equal a, got similarity {}",
            recovered.similarity(&a));
    }

    /// bundle(a, b) == bundle(b, a) — bundle is commutative
    #[test]
    fn bundle_is_commutative(a in arb_hdc_vector(), b in arb_hdc_vector()) {
        let ab = HdcVector::bundle(&[&a, &b]);
        let ba = HdcVector::bundle(&[&b, &a]);
        prop_assert_eq!(ab, ba);
    }

    /// similarity with self is always 1.0
    #[test]
    fn self_similarity_is_one(v in arb_hdc_vector()) {
        prop_assert!((v.similarity(&v) - 1.0).abs() < 1e-6);
    }

    /// similarity is in [0, 1]
    #[test]
    fn similarity_in_unit_range(a in arb_hdc_vector(), b in arb_hdc_vector()) {
        let sim = a.similarity(&b);
        prop_assert!(sim >= 0.0 && sim <= 1.0, "similarity {} out of [0,1]", sim);
    }

    /// similarity is symmetric
    #[test]
    fn similarity_is_symmetric(a in arb_hdc_vector(), b in arb_hdc_vector()) {
        let ab = a.similarity(&b);
        let ba = b.similarity(&a);
        prop_assert!((ab - ba).abs() < 1e-6);
    }

    /// bytes roundtrip: from_bytes(to_bytes(v)) == v
    #[test]
    fn bytes_roundtrip(v in arb_hdc_vector()) {
        let bytes = v.to_bytes();
        let recovered = HdcVector::from_bytes(&bytes);
        prop_assert_eq!(v, recovered);
    }

    /// bind with zero vector is identity
    #[test]
    fn bind_with_zero_is_identity(v in arb_hdc_vector()) {
        let zero = HdcVector::zeros();
        let result = v.bind(&zero);
        prop_assert_eq!(v, result);
    }

    /// permute(0) is identity
    #[test]
    fn permute_zero_is_identity(v in arb_hdc_vector()) {
        prop_assert_eq!(v, v.permute(0));
    }

    /// permute preserves popcount (number of set bits)
    #[test]
    fn permute_preserves_popcount(v in arb_hdc_vector(), n in 0usize..10240) {
        let permuted = v.permute(n);
        let orig_sim = v.similarity(&HdcVector::zeros());
        let perm_sim = permuted.similarity(&HdcVector::zeros());
        // Same distance from zero means same popcount
        prop_assert!((orig_sim - perm_sim).abs() < 1e-6);
    }
}
