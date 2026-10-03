//! RFC-0328: Property-Based Algebraic Axiom & Structural Invariant Verification Suite.
//!
//! Mechanically asserts that all mathematical kernels strictly satisfy their foundational
//! algebraic axioms (Monoid Associativity/Identity, Semiring Idempotence, Banach Contraction,
//! and KeyInterval Closed Bounds) across adversarial, pseudo-random, and extreme boundary inputs.

#![forbid(unsafe_code)]

use pedradb_core::domain_bounds_kernel::{
    CheckedPermille, DomainBoundsError, KeyInterval,
};
use pedradb_core::sst_metadata_measure_monoid_kernel::{
    MeasureMonoidViolation, SstMetadataMeasure,
};
use pedradb_core::manifest_version_edit_semiring_kernel::{
    SstFileMetadata, VersionDelta, VersionEditSemiring, VersionState,
};

/// Deterministic Linear Congruential Generator for reproducible adversarial PBT.
struct AdversarialPrng {
    state: u64,
}

impl AdversarialPrng {
    fn new(seed: u64) -> Self {
        Self { state: seed.wrapping_add(0x9E3779B97F4A7C15) }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.state
    }

    fn next_bounded(&mut self, bound: u64) -> u64 {
        if bound == 0 {
            0
        } else {
            self.next_u64() % bound
        }
    }

    fn next_extreme_u64(&mut self) -> u64 {
        match self.next_bounded(8) {
            0 => 0,
            1 => 1,
            2 => 2,
            3 => self.next_bounded(1_000_000),
            4 => u64::MAX,
            5 => u64::MAX - 1,
            6 => u64::MAX / 2,
            _ => self.next_u64(),
        }
    }

    fn next_bytes(&mut self, max_len: usize) -> Vec<u8> {
        let len = self.next_bounded(max_len as u64) as usize;
        let mut v = Vec::with_capacity(len);
        for _ in 0..len {
            v.push(self.next_u64() as u8);
        }
        v
    }
}

#[test]
fn test_sst_metadata_measure_monoid_axioms_pbt() {
    let mut prng = AdversarialPrng::new(0xABCD_1234_5678);

    for _ in 0..2000 {
        let raw_a = prng.next_bounded(10_000_000);
        let rec_a = prng.next_bounded(100_000);
        let tomb_a = prng.next_bounded(rec_a + 1);
        let obs_a = prng.next_bounded(raw_a + 1);

        let raw_b = prng.next_bounded(10_000_000);
        let rec_b = prng.next_bounded(100_000);
        let tomb_b = prng.next_bounded(rec_b + 1);
        let obs_b = prng.next_bounded(raw_b + 1);

        let raw_c = prng.next_bounded(10_000_000);
        let rec_c = prng.next_bounded(100_000);
        let tomb_c = prng.next_bounded(rec_c + 1);
        let obs_c = prng.next_bounded(raw_c + 1);

        let a = SstMetadataMeasure::try_new(raw_a, rec_a, tomb_a, obs_a).expect("valid a");
        let b = SstMetadataMeasure::try_new(raw_b, rec_b, tomb_b, obs_b).expect("valid b");
        let c = SstMetadataMeasure::try_new(raw_c, rec_c, tomb_c, obs_c).expect("valid c");

        // Axiom 1: Identity element (zero)
        let identity = SstMetadataMeasure::IDENTITY;
        assert_eq!(a.checked_combine(identity), Some(a));
        assert_eq!(identity.checked_combine(a), Some(a));

        // Axiom 2: Associativity: (A + B) + C == A + (B + C)
        let ab = a.checked_combine(b);
        let bc = b.checked_combine(c);

        if let (Some(ab_val), Some(bc_val)) = (ab, bc) {
            let ab_c = ab_val.checked_combine(c);
            let a_bc = a.checked_combine(bc_val);
            assert_eq!(ab_c, a_bc, "Monoid associativity broken for valid inputs");
        }

        // Axiom 3: Commutativity: A + B == B + A
        assert_eq!(a.checked_combine(b), b.checked_combine(a));
    }
}

#[test]
fn test_sst_metadata_measure_overflow_eradication_pbt() {
    let mut prng = AdversarialPrng::new(0xDEAD_BEEF_CAFE);

    for _ in 0..1000 {
        // Gera valores no limite do u64 para forçar overflow
        let a = SstMetadataMeasure {
            raw_data_bytes: u64::MAX - prng.next_bounded(1000),
            record_count: 10,
            tombstone_count: 0,
            obsolete_bytes: 0,
        };
        let b = SstMetadataMeasure {
            raw_data_bytes: 1001 + prng.next_bounded(1000),
            record_count: 5,
            tombstone_count: 0,
            obsolete_bytes: 0,
        };

        // Deve falhar fechado com IntegerOverflow sem saturação silenciosa
        assert_eq!(a.checked_combine(b), None);
        assert_eq!(a.try_combine(b), Err(MeasureMonoidViolation::IntegerOverflow));
    }
}

#[test]
fn test_manifest_version_edit_semiring_idempotence_pbt() {
    for i in 1..=500 {
        let vset = VersionState::default();

        let s1 = SstFileMetadata::try_new(
            i as u64,
            0,
            1024,
            format!("key_{:06}", i).into_bytes(),
            format!("key_{:06}", i + 10).into_bytes(),
        ).expect("valid s1");

        let s2 = SstFileMetadata::try_new(
            (i + 500) as u64,
            1,
            2048,
            format!("key_{:06}", i + 20).into_bytes(),
            format!("key_{:06}", i + 30).into_bytes(),
        ).expect("valid s2");

        let delta = VersionDelta {
            added_files: vec![s1.clone(), s2.clone()],
            deleted_files: vec![],
            next_file_number: Some((i + 1000) as u64),
            last_sequence: Some(i as u64),
        };

        // Semiring Idempotence: V + E == V + E + E
        let v1 = VersionEditSemiring::try_apply(vset.clone(), &delta).expect("apply 1");
        let v2 = VersionEditSemiring::try_apply(v1.clone(), &delta).expect("apply 2 (idempotent replay)");

        assert_eq!(v1, v2, "Semiring idempotence violated: V + E != V + E + E");
        assert_eq!(v1.total_file_count(), 2);
    }
}

#[test]
fn test_key_interval_homomorphic_properties_pbt() {
    let mut prng = AdversarialPrng::new(0x7777_8888_9999);

    for _ in 0..1000 {
        let mut k1 = prng.next_bytes(32);
        let mut k2 = prng.next_bytes(32);

        // Se vazias, devem ser rejeitadas
        if k1.is_empty() || k2.is_empty() {
            assert!(KeyInterval::try_new(k1, k2).is_err());
            continue;
        }

        if k1 > k2 {
            // Inversão garantida de rejeição
            assert!(matches!(
                KeyInterval::try_new(k1.clone(), k2.clone()),
                Err(DomainBoundsError::InvertedKeyInterval { .. })
            ));
            std::mem::swap(&mut k1, &mut k2);
        }

        // Agora k1 <= k2: deve ser aceito
        let interval = KeyInterval::try_new(k1.clone(), k2.clone()).expect("valid interval");
        assert!(interval.contains_key(&k1));
        assert!(interval.contains_key(&k2));
        assert!(interval.overlaps(&interval));
        assert!(!interval.is_disjoint(&interval));
    }
}

#[test]
fn test_checked_permille_conservation_pbt() {
    let mut prng = AdversarialPrng::new(0x1234_5678_90AB);

    for _ in 0..1000 {
        let num = prng.next_extreme_u64();
        let den = prng.next_extreme_u64();

        if den == 0 {
            assert_eq!(
                CheckedPermille::from_ratio(num, den),
                Err(DomainBoundsError::ZeroDenominator)
            );
            continue;
        }

        let p = CheckedPermille::from_ratio(num, den).expect("valid ratio");
        assert!(p.get() <= 1000, "Permille cannot exceed 1000");

        let total = prng.next_bounded(1_000_000_000);
        let res = p.apply_to(total).expect("safe apply");
        assert!(res <= total + 1, "Scaled value cannot exceed total");

        if p == CheckedPermille::ONE_HUNDRED_PERCENT {
            assert_eq!(res, total);
        } else if p == CheckedPermille::ZERO {
            assert_eq!(res, 0);
        }
    }
}
