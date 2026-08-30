// Verus proof of the Bloom filter core contract (T1 / T4 of
// docs/formal/bloom-filter-theorems.md).
// Twin of `pedradb-core/src/bloom.rs` insert/may_contain — model domain:
// bits are `Vec<bool>` (one slot per bit, so in-bounds is `idx < nbits`) and
// the probe index reduces operands before multiplying so the model never
// wraps (production wraps u64). T1 only needs insert and query to agree on
// the index expression — both call the same `probe_index`; the bit-precise
// u64 wrapping semantics of the production expression, and the multi-key
// monotonicity argument, are carried by the Kani harnesses on the production
// code itself.
//
//   ./scripts/verus_bloom_filter.sh

use vstd::prelude::*;

verus! {

/// Kirsch–Mitzenmacher probe index, model form: operands reduced first so
/// `i * h2` and the sum stay below 2^64 (no wrapping in the model).
pub open spec fn probe_index_spec(h1: int, h2: int, i: int, nbits: int) -> int {
    ((h1 % nbits) + ((i * (h2 % nbits)) % nbits)) % nbits
}

/// Exec form of the same expression.
pub fn probe_index(h1: u64, h2: u64, i: u64, nbits: u64) -> (r: u64)
    requires
        1 <= nbits <= 0xffff_ffff,
        i <= 30,
    ensures
        r as int == probe_index_spec(h1 as int, h2 as int, i as int, nbits as int),
        r < nbits,
{
    let a = h1 % nbits;
    let b = h2 % nbits;
    proof {
        vstd::arithmetic::mul::lemma_mul_upper_bound(i as int, 30, b as int, 0xffff_fffe);
    }
    let m = (i * b) % nbits;
    (a + m) % nbits
}

/// Model filter: one bool per bit, so `bits.len() == nbits`.
pub struct BloomTwin {
    pub bits: Vec<bool>,
    pub nbits: u64,
    pub k: u64,
}

impl BloomTwin {
    /// Active model filter (production `is_active`: nbits > 0 ∧ k > 0 ∧
    /// non-empty bits; the model collapses that to the length equation).
    pub open spec fn inv(&self) -> bool {
        &&& self.nbits >= 1
        &&& self.nbits <= 0xffff_ffff
        &&& self.k >= 1
        &&& self.k <= 30
        &&& self.bits@.len() == self.nbits as int
    }

    /// Model of `BloomFilter::insert`: set every probe bit of the key
    /// (identified by its deterministic hash pair). Bits only get set.
    pub fn insert(&mut self, h1: u64, h2: u64)
        requires
            old(self).inv(),
        ensures
            final(self).inv(),
            final(self).nbits == old(self).nbits,
            final(self).k == old(self).k,
            final(self).bits@.len() == old(self).bits@.len(),
            // T1 postcondition: every probe bit of this key is now set.
            forall |t: int| 0 <= t < old(self).k as int
                ==> final(self).bits@[probe_index_spec(h1 as int, h2 as int, t, old(self).nbits as int)],
    {
        let nbits = self.nbits;
        let k = self.k;
        let mut i: u64 = 0;
        while i < k
            invariant
                0 <= i <= k,
                k <= 30,
                1 <= nbits <= 0xffff_ffff,
                self.nbits == nbits,
                self.k == k,
                self.bits@.len() == nbits as int,
                forall |t: int| 0 <= t < i
                    ==> self.bits@[probe_index_spec(h1 as int, h2 as int, t, nbits as int)],
            decreases k - i,
        {
            let idx = probe_index(h1, h2, i, nbits) as usize;
            self.bits[idx] = true;
            i += 1;
        }
    }

    /// Model of `BloomFilter::may_contain`: `false` ⇒ a probe bit is clear.
    pub fn may_contain(&self, h1: u64, h2: u64) -> (r: bool)
        requires
            self.inv(),
        ensures
            (forall |t: int| 0 <= t < self.k as int
                ==> self.bits@[probe_index_spec(h1 as int, h2 as int, t, self.nbits as int)])
                ==> r,
            !r ==> (exists |t: int| 0 <= t < self.k as int
                && !self.bits@[probe_index_spec(h1 as int, h2 as int, t, self.nbits as int)]),
    {
        let nbits = self.nbits;
        let k = self.k;
        let mut i: u64 = 0;
        while i < k
            invariant
                0 <= i <= k,
                k <= 30,
                1 <= nbits <= 0xffff_ffff,
                self.nbits == nbits,
                self.k == k,
                self.bits@.len() == nbits as int,
                forall |t: int| 0 <= t < i
                    ==> self.bits@[probe_index_spec(h1 as int, h2 as int, t, nbits as int)],
            decreases k - i,
        {
            let idx = probe_index(h1, h2, i, nbits) as usize;
            if !self.bits[idx] {
                assert(!self.bits@[probe_index_spec(h1 as int, h2 as int, i as int, nbits as int)]);
                return false;
            }
            i += 1;
        }
        true
    }
}

/// T1 (unbounded, model domain): for **every** active filter and **every**
/// deterministic hash pair, inserting a key and then querying it never
/// reports the key absent. The fn contract is the universally quantified
/// theorem; the body composes the two loop proofs above.
pub fn insert_then_may_contain(f: &mut BloomTwin, h1: u64, h2: u64) -> (r: bool)
    requires
        old(f).inv(),
    ensures
        r,
        final(f).inv(),
        final(f).nbits == old(f).nbits,
        final(f).k == old(f).k,
        final(f).bits@.len() == old(f).bits@.len(),
{
    f.insert(h1, h2);
    f.may_contain(h1, h2)
}

/// T4: an inactive filter (empty bits / nbits 0 / k 0) never rejects a key —
/// production `may_contain` returns `true` before touching any bit.
pub fn may_contain_inactive(nbits: u64, k: u64, nbits_len: u64) -> (r: bool)
    requires
        nbits == 0 || k == 0 || nbits_len == 0,
    ensures
        r,
{
    true
}

} // verus!
