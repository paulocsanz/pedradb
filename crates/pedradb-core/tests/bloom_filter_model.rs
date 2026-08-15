//! Finite-domain exhaustive model for the **real** [`pedradb_core::BloomFilter`]
//! (theorems T1–T4 of `docs/formal/bloom-filter-theorems.md`).
//!
//! Faithfulness: these tests call the production `insert` / `may_contain` /
//! `encode` / `decode` directly — no paraphrase. The domain is small enough to
//! be exhaustive: every key over alphabet {0x00, 0x01, 0xfe, 0xff} up to length
//! 3 (85 keys) × a grid of `with_capacity` shapes. The teeth tests assert the
//! T1 mutants (`may_contain_mut_*`) actually produce a false negative
//! somewhere in the same domain — otherwise the properties above bite nothing.
//!
//! The unbounded ∀ versions of T1–T4 are carried by the Kani harnesses
//! (`#[cfg(kani)] mod kani_proofs` in `src/bloom.rs`), the Verus twin
//! (`verus/bloom_filter.rs`), and the Aeneas/Lean extract (`formal/aeneas/`).

use pedradb_core::bloom::{may_contain_mut_extra_probe, may_contain_mut_hash_mismatch};
use pedradb_core::BloomFilter;

const ALPHABET: [u8; 4] = [0x00, 0x01, 0xfe, 0xff];

/// Every key over `ALPHABET` with length 0..=max_len (1 + 4 + 16 + 64 = 85).
fn all_keys(max_len: usize) -> Vec<Vec<u8>> {
    let mut out = vec![Vec::new()];
    for len in 1..=max_len {
        let mut idx = vec![0usize; len];
        'outer: loop {
            out.push(idx.iter().map(|&i| ALPHABET[i]).collect());
            // odometer increment; full overflow = done with this length
            for pos in (0..len).rev() {
                idx[pos] += 1;
                if idx[pos] < ALPHABET.len() {
                    continue 'outer;
                }
                idx[pos] = 0;
            }
            break;
        }
    }
    out
}

/// (n_keys, bits_per_key) grid — small/saturated and large/sparse shapes.
fn grid() -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for n_keys in [1usize, 8, 85] {
        for bpk in [1usize, 2, 5, 10, 20] {
            out.push((n_keys, bpk));
        }
    }
    out
}

/// Deterministic xorshift64* for the bounded fuzz smoke (no external deps).
struct Xorshift(u64);

impl Xorshift {
    fn next(&mut self) -> u64 {
        let mut s = self.0;
        s ^= s >> 12;
        s ^= s << 25;
        s ^= s >> 27;
        self.0 = s;
        s.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn byte_vec(&mut self, len: usize) -> Vec<u8> {
        (0..len).map(|_| (self.next() & 0xff) as u8).collect()
    }
}

/// T1: no false negatives — every inserted key is still reported present.
#[test]
fn t1_no_false_negatives_exhaustive() {
    let keys = all_keys(3);
    assert_eq!(keys.len(), 85);
    for &(n_keys, bpk) in &grid() {
        let mut f = BloomFilter::with_capacity(n_keys, bpk);
        for k in &keys {
            f.insert(k);
        }
        for k in &keys {
            assert!(
                f.may_contain(k),
                "T1 false negative: n_keys={n_keys} bpk={bpk} key={k:?}"
            );
        }
    }
}

/// T2: `decode(encode(f))` reproduces the filter (structural) and keeps every
/// membership decision.
#[test]
fn t2_roundtrip_preserves_decisions_exhaustive() {
    let keys = all_keys(3);
    for &(n_keys, bpk) in &grid() {
        let mut f = BloomFilter::with_capacity(n_keys, bpk);
        for k in &keys {
            f.insert(k);
        }
        let g = BloomFilter::decode(&f.encode()).expect("self-produced filter must decode");
        assert_eq!(g, f, "T2 structural: n_keys={n_keys} bpk={bpk}");
        assert_eq!(g.bit_count(), f.bit_count());
        assert_eq!(g.hash_count(), f.hash_count());
        for k in &keys {
            assert_eq!(
                g.may_contain(k),
                f.may_contain(k),
                "T2 decision: n_keys={n_keys} bpk={bpk} key={k:?}"
            );
        }
    }
}

/// T3 teeth (regression form of the Kani bound): every header the writer can
/// produce decodes back — including the probe-count clamp extremes.
#[test]
fn t3_writer_headers_decode_exhaustive() {
    for bits_per_key in 1..=64usize {
        for n_keys in [1usize, 2, 7, 64] {
            let f = BloomFilter::with_capacity(n_keys, bits_per_key);
            let g = BloomFilter::decode(&f.encode()).expect("writer header must decode");
            assert_eq!(g, f);
            assert!(f.hash_count() >= 1 && f.hash_count() <= pedradb_core::MAX_K);
        }
    }
}

/// T4: an inactive filter never rejects any key.
#[test]
fn t4_inactive_never_rejects_exhaustive() {
    let keys = all_keys(3);
    let inactives = [
        BloomFilter::always_true(),
        BloomFilter::with_capacity(0, 10),
        BloomFilter::with_capacity(10, 0),
    ];
    for f in &inactives {
        assert!(!f.is_active());
        for k in &keys {
            assert!(f.may_contain(k), "T4: inactive filter rejected {k:?}");
        }
    }
}

/// Teeth: the extra-probe mutant must produce at least one false negative on
/// the same domain (otherwise the T1 property bites nothing).
#[test]
fn teeth_extra_probe_mutant_bites() {
    let keys = all_keys(3);
    let mut bites = 0usize;
    for &(n_keys, bpk) in &grid() {
        let mut f = BloomFilter::with_capacity(n_keys, bpk);
        for k in &keys {
            f.insert(k);
        }
        for k in &keys {
            if !may_contain_mut_extra_probe(&f, k) {
                bites += 1;
            }
        }
    }
    assert!(
        bites > 0,
        "mutant survived the whole domain — T1 has no teeth"
    );
}

/// Teeth: the hash-mismatch mutant must produce at least one false negative.
#[test]
fn teeth_hash_mismatch_mutant_bites() {
    let keys = all_keys(3);
    let mut bites = 0usize;
    for &(n_keys, bpk) in &grid() {
        let mut f = BloomFilter::with_capacity(n_keys, bpk);
        for k in &keys {
            f.insert(k);
        }
        for k in &keys {
            if !may_contain_mut_hash_mismatch(&f, k) {
                bites += 1;
            }
        }
    }
    assert!(
        bites > 0,
        "mutant survived the whole domain — T1 has no teeth"
    );
}

/// Bounded fuzz smoke (deterministic seed): T1 + T2 + T4 on random shapes.
#[test]
fn fuzz_smoke_t1_t2_t4() {
    let mut rng = Xorshift(0x5EED_0000_00FA_CE5);
    for _ in 0..500 {
        let n_keys = (rng.next() % 300) as usize;
        let bpk = 1 + (rng.next() % 32) as usize;
        let mut f = BloomFilter::with_capacity(n_keys, bpk);
        let n_ins = 1 + (rng.next() % 24) as usize;
        let mut keys = Vec::with_capacity(n_ins);
        for _ in 0..n_ins {
            let len = (rng.next() % 24) as usize;
            keys.push(rng.byte_vec(len));
        }
        for k in &keys {
            f.insert(k);
        }
        for k in &keys {
            assert!(f.may_contain(k), "fuzz T1 false negative: key={k:?}");
        }
        let g = BloomFilter::decode(&f.encode()).expect("fuzz roundtrip must decode");
        assert_eq!(g, f);
        for k in &keys {
            assert!(g.may_contain(k), "fuzz T2 lost member: key={k:?}");
        }
    }
}
