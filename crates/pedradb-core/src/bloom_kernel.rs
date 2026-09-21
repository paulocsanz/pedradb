//! Bloom filter for SST point-lookup negative caching (RocksDB/Pebble class).
//!
//! On-disk SSTs embed a filter so [`crate::sst::SstTable::get`] can skip files
//! that cannot contain a key. False positives are allowed; false negatives are not.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

static BLOOM_DECODES: AtomicU64 = AtomicU64::new(0);
static FILTER_PART_LOADS: AtomicU64 = AtomicU64::new(0);
static FILTER_BYTES_LOADED: AtomicU64 = AtomicU64::new(0);

/// On-disk sentinel `nbits` for a partitioned filter (RFC-0236).
pub const FILTER_PART_MAGIC: u32 = 0xFFFF_FFFE;

/// RFC-0235: bloom payloads decoded from disk (pin must keep this flat).
#[must_use]
pub fn bloom_decode_count() -> u64 {
    BLOOM_DECODES.load(Ordering::Relaxed)
}

/// RFC-0235: zero the decode counter (tests).
pub fn reset_bloom_decode_count() {
    BLOOM_DECODES.store(0, Ordering::Relaxed);
}

/// RFC-0236: partitions decoded on the miss path (pin/lazy load).
#[must_use]
pub fn filter_part_load_count() -> u64 {
    FILTER_PART_LOADS.load(Ordering::Relaxed)
}

/// RFC-0236: zero partition-load counter (tests).
pub fn reset_filter_part_load_count() {
    FILTER_PART_LOADS.store(0, Ordering::Relaxed);
    FILTER_BYTES_LOADED.store(0, Ordering::Relaxed);
}

/// RFC-0236 PHASE: bytes of filter payload decoded (one partition at a time).
#[must_use]
pub fn filter_bytes_loaded() -> u64 {
    FILTER_BYTES_LOADED.load(Ordering::Relaxed)
}

/// Default bits per key (~1% false-positive rate with k≈7).
pub const DEFAULT_BITS_PER_KEY: usize = 10;

/// Monkey (R006): more bits/key on smaller runs so \(\sum\mathrm{FPR}\) does
/// not grow with \(L\). Integer thresholds; no autotune \(T\).
#[must_use]
pub fn bits_per_key_for_run(n_keys: u64) -> usize {
    if n_keys == 0 {
        return DEFAULT_BITS_PER_KEY;
    }
    if n_keys < 4096 {
        16
    } else if n_keys < 65536 {
        12
    } else {
        DEFAULT_BITS_PER_KEY
    }
}

/// AS-IS: uniform 10 bits/key regardless of run size.
#[must_use]
pub fn bits_per_key_for_run_as_is(_n_keys: u64) -> usize {
    DEFAULT_BITS_PER_KEY
}

/// Hard upper bound on probe count `k` written by [`BloomFilter::with_capacity`].
///
/// F166: on-disk `k` is untrusted; `may_contain` loops `k` times, so a corrupt
/// value near `u32::MAX` turns every point lookup into minutes of CPU.
pub const MAX_K: u32 = 30;

/// Fail-closed validation of an on-disk bloom header (F166 kernel).
///
/// `residual` is the payload left after the 12-byte header. Accepting implies
/// bounded probe work (`1 <= k <= MAX_K`) and a bits array that both covers
/// `nbits` and fits the buffer.
#[must_use]
pub fn bloom_header_ok(nbits: u32, k: u32, nbytes: u32, residual: u64) -> bool {
    (1..=MAX_K).contains(&k)
        && u64::from(nbytes) >= u64::from(nbits).div_ceil(8)
        && u64::from(nbytes) <= residual
}

/// AS-IS F166: no probe-count bound (accepts `k` up to `u32::MAX`).
#[must_use]
pub fn bloom_header_ok_as_is(nbits: u32, _k: u32, nbytes: u32, residual: u64) -> bool {
    u64::from(nbytes) >= u64::from(nbits).div_ceil(8) && u64::from(nbytes) <= residual
}

/// Saturating `a * b` without `saturating_mul` (Aeneas extracts that as an axiom).
fn saturating_product(a: usize, b: usize) -> usize {
    if b != 0 && a > usize::MAX / b {
        usize::MAX
    } else {
        a * b
    }
}

fn at_least_64(x: usize) -> usize {
    if x < 64 {
        64
    } else {
        x
    }
}

fn cap_u32(x: usize) -> u32 {
    if x > u32::MAX as usize {
        u32::MAX
    } else {
        #[allow(clippy::cast_possible_truncation)]
        {
            x as u32
        }
    }
}

/// `k ≈ bits_per_key * ln(2) ≈ bits_per_key * 0.69`, clamped to `[1, 30]`.
fn k_from_bits_per_key(bits_per_key: usize) -> u32 {
    let raw = if bits_per_key > usize::MAX / 69 {
        30
    } else {
        (bits_per_key * 69) / 100
    };
    if raw < 1 {
        1
    } else if raw > 30 {
        30
    } else {
        #[allow(clippy::cast_possible_truncation)]
        {
            raw as u32
        }
    }
}

/// `(nbits + 7) / 8` — same as `div_ceil(8)` for unsigned, axiom-free extract.
fn nbytes_for_nbits(nbits: u32) -> usize {
    (nbits as usize + 7) / 8
}

/// Double-hash Bloom filter (Kirsch–Mitzenmacher).
///
/// RFC-0236: `nparts > 1` is a partitioned filter (Fjall 3 / Rocks). The
/// miss path decodes **one** part via [`crate::filter_partition`].
#[derive(Debug)]
pub struct BloomFilter {
    bits: Arc<Vec<u8>>,
    /// Number of bits in the filter.
    nbits: u32,
    /// Number of hash probes per key.
    k: u32,
    nparts: u32,
    encoded_parts: Arc<Vec<Vec<u8>>>,
    loaded: Arc<Mutex<Vec<Option<Box<BloomFilter>>>>>,
    /// Read-side lazy parts (lock-free after first load). Write path uses `loaded`.
    parts_once: Arc<Vec<OnceLock<BloomFilter>>>,
}

impl Clone for BloomFilter {
    fn clone(&self) -> Self {
        Self {
            bits: Arc::clone(&self.bits),
            nbits: self.nbits,
            k: self.k,
            nparts: self.nparts,
            encoded_parts: Arc::clone(&self.encoded_parts),
            loaded: Arc::clone(&self.loaded),
            parts_once: Arc::clone(&self.parts_once),
        }
    }
}

impl PartialEq for BloomFilter {
    fn eq(&self, other: &Self) -> bool {
        self.nbits == other.nbits
            && self.k == other.k
            && self.bits == other.bits
            && self.nparts == other.nparts
            && self.encoded_parts == other.encoded_parts
    }
}
impl Eq for BloomFilter {}

impl BloomFilter {
    /// Empty filter that always returns [`true`] for `may_contain` (no filtering).
    #[must_use]
    pub fn always_true() -> Self {
        Self {
            bits: Arc::new(Vec::new()),
            nbits: 0,
            k: 0,
            nparts: 0,
            encoded_parts: Arc::new(Vec::new()),
            loaded: Arc::new(Mutex::new(Vec::new())),
            parts_once: Arc::new(Vec::new()),
        }
    }

    /// Whether this filter can reject keys (non-empty).
    ///
    /// RFC-0030 P2: `len() != 0` rather than `!is_empty()` — `Vec::is_empty`
    /// extracts as an Aeneas axiom; `len` is in the Lean std.
    #[must_use]
    pub fn is_active(&self) -> bool {
        if self.nparts > 1 {
            return true;
        }
        #[allow(clippy::len_zero)]
        {
            self.nbits > 0 && self.k > 0 && self.bits.len() != 0
        }
    }

    /// Build a filter sized for approximately `n_keys` insertions.
    ///
    /// RFC-0030 P2: no `saturating_mul` / `max` / `min` / `clamp` /
    /// `try_from` / `div_ceil` — those extract as axioms or fail to
    /// typecheck in Lean (`Ord.max.default`). The integer form is the
    /// same: `nbits = min(max(n_keys * bpk, 64), u32::MAX)`,
    /// `k = clamp((bpk * 69) / 100, 1, 30)`, `nbytes = (nbits + 7) / 8`.
    #[must_use]
    pub fn with_capacity(n_keys: usize, bits_per_key: usize) -> Self {
        if n_keys == 0 || bits_per_key == 0 {
            return Self::always_true();
        }
        let nbits = cap_u32(at_least_64(saturating_product(n_keys, bits_per_key)));
        let k = k_from_bits_per_key(bits_per_key);
        let nbytes = nbytes_for_nbits(nbits);
        Self {
            bits: Arc::new(vec![0u8; nbytes]),
            nbits,
            k,
            nparts: 1,
            encoded_parts: Arc::new(Vec::new()),
            loaded: Arc::new(Mutex::new(Vec::new())),
            parts_once: Arc::new(Vec::new()),
        }
    }

    /// RFC-0236: `nparts` live filters for insert, encoded as partitions.
    #[must_use]
    pub fn with_partitions(n_keys: usize, bits_per_key: usize, nparts: u32) -> Self {
        if nparts <= 1 {
            return Self::with_capacity(n_keys, bits_per_key);
        }
        let per = n_keys / nparts as usize + 1;
        let mut loaded = Vec::with_capacity(nparts as usize);
        for _ in 0..nparts {
            loaded.push(Some(Box::new(Self::with_capacity(per, bits_per_key))));
        }
        Self {
            bits: Arc::new(Vec::new()),
            nbits: 0,
            k: 0,
            nparts,
            encoded_parts: Arc::new(Vec::new()),
            loaded: Arc::new(Mutex::new(loaded)),
            parts_once: Arc::new(Vec::new()),
        }
    }

    /// Insert a user key.
    ///
    /// RFC-0030 P2: `while` + shared [`probe_bit`], not `for 0..k` — range
    /// iterators extract to `IteratorRange` and block the Lean T1 loop
    /// proof. Isolated used the same rewrite (`starts_with` → byte loop).
    pub fn insert(&mut self, key: &[u8]) {
        if self.nparts > 1 {
            let (h1, _) = hash_pair(key);
            let i = crate::filter_partition_kernel::filter_partition(h1, self.nparts) as usize;
            let mut g = self.loaded.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(Some(p)) = g.get_mut(i) {
                p.insert(key);
            }
            return;
        }
        if !self.is_active() {
            return;
        }
        let (h1, h2) = hash_pair(key);
        let nbits = u64::from(self.nbits);
        let mut i = 0u32;
        while i < self.k {
            set_bit(
                Arc::make_mut(&mut self.bits).as_mut_slice(),
                bit_index(probe_bit(h1, h2, i, nbits)),
            );
            i += 1;
        }
    }

    /// AS-IS T1: skip setting probe bits — a written key can false-negative.
    pub fn insert_as_is(&mut self, _key: &[u8]) {}

    /// `false` ⇒ key is definitely absent; `true` ⇒ maybe present.
    #[must_use]
    pub fn may_contain(&self, key: &[u8]) -> bool {
        if self.nparts > 1 {
            return self.may_contain_part(key);
        }
        if !self.is_active() {
            return true;
        }
        let (h1, h2) = hash_pair(key);
        let nbits = u64::from(self.nbits);
        let mut i = 0u32;
        while i < self.k {
            if !test_bit(&self.bits, bit_index(probe_bit(h1, h2, i, nbits))) {
                return false;
            }
            i += 1;
        }
        true
    }

    /// AS-IS T1: query probes `k+1` bits (false negative when the extra bit is clear).
    #[must_use]
    pub fn may_contain_as_is(&self, key: &[u8]) -> bool {
        may_contain_mut_extra_probe(self, key)
    }

    fn may_contain_part(&self, key: &[u8]) -> bool {
        let (h1, _) = hash_pair(key);
        let i = crate::filter_partition_kernel::filter_partition(h1, self.nparts) as usize;
        if let Some(slot) = self.parts_once.get(i) {
            let part = slot.get_or_init(|| {
                let raw = self.encoded_parts.get(i).cloned().unwrap_or_default();
                FILTER_PART_LOADS.fetch_add(1, Ordering::Relaxed);
                FILTER_BYTES_LOADED.fetch_add(raw.len() as u64, Ordering::Relaxed);
                Self::decode_mono(&raw).unwrap_or_else(|_| Self::always_true())
            });
            return part.may_contain(key);
        }
        let mut g = self.loaded.lock().unwrap_or_else(|e| e.into_inner());
        if i >= g.len() {
            g.resize_with(self.nparts as usize, || None);
        }
        if g[i].is_none() {
            let Some(raw) = self.encoded_parts.get(i) else {
                return true;
            };
            FILTER_PART_LOADS.fetch_add(1, Ordering::Relaxed);
            FILTER_BYTES_LOADED.fetch_add(raw.len() as u64, Ordering::Relaxed);
            let decoded = Self::decode_mono(raw).unwrap_or_else(|_| Self::always_true());
            g[i] = Some(Box::new(decoded));
        }
        g[i].as_ref().is_none_or(|p| p.may_contain(key))
    }

    /// Encode for SST trailer: `nbits u32 | k u32 | nbytes u32 | bits`.
    /// Partitioned: `FILTER_PART_MAGIC | nparts | nbytes | part encodings`.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        if self.nparts > 1 {
            return self.encode_partitioned();
        }
        let mut out = Vec::with_capacity(12 + self.bits.len());
        out.extend_from_slice(&self.nbits.to_le_bytes());
        out.extend_from_slice(&self.k.to_le_bytes());
        let nbytes = u32::try_from(self.bits.len()).unwrap_or(u32::MAX);
        out.extend_from_slice(&nbytes.to_le_bytes());
        out.extend_from_slice(&self.bits);
        out
    }

    fn encode_partitioned(&self) -> Vec<u8> {
        let g = self.loaded.lock().unwrap_or_else(|e| e.into_inner());
        let mut body = Vec::new();
        if !g.is_empty() {
            for p in g.iter() {
                let enc = p.as_ref().map_or_else(Vec::new, |x| x.encode());
                let n = enc.len() as u32;
                body.extend_from_slice(&n.to_le_bytes());
                body.extend_from_slice(&enc);
            }
        } else {
            for enc in self.encoded_parts.iter() {
                let n = enc.len() as u32;
                body.extend_from_slice(&n.to_le_bytes());
                body.extend_from_slice(enc);
            }
        }
        drop(g);
        let mut out = Vec::with_capacity(12 + body.len());
        out.extend_from_slice(&FILTER_PART_MAGIC.to_le_bytes());
        out.extend_from_slice(&self.nparts.to_le_bytes());
        let nbytes = u32::try_from(body.len()).unwrap_or(u32::MAX);
        out.extend_from_slice(&nbytes.to_le_bytes());
        out.extend_from_slice(&body);
        out
    }

    /// Decode filter bytes. Empty / zero-sized → always-true.
    ///
    /// # Errors
    /// Truncated or inconsistent lengths, or an out-of-bound probe count
    /// (`k > MAX_K`, F166 — fail closed rather than loop billions of times).
    pub fn decode(buf: &[u8]) -> Result<Self, String> {
        BLOOM_DECODES.fetch_add(1, Ordering::Relaxed);
        if buf.is_empty() {
            return Ok(Self::always_true());
        }
        if buf.len() < 12 {
            return Err(format!("bloom too short: {} bytes", buf.len()));
        }
        let nbits = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
        let k = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
        let nbytes = u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]);
        let nbytes_us = nbytes as usize;
        if 12 + nbytes_us > buf.len() {
            return Err("bloom bits truncated".into());
        }
        if nbits == FILTER_PART_MAGIC {
            return Self::decode_partitioned(&buf[12..12 + nbytes_us], k);
        }
        if nbits == 0 || k == 0 || nbytes == 0 {
            return Ok(Self::always_true());
        }
        // F166: bound probes and bits coverage before touching the data.
        let residual = (buf.len() - 12) as u64;
        if !bloom_header_ok(nbits, k, nbytes, residual) {
            return Err(format!(
                "bloom header invalid: nbits {nbits} k {k} nbytes {nbytes} (max k {MAX_K})"
            ));
        }
        let bits = buf[12..12 + nbytes_us].to_vec();
        Ok(Self {
            bits: Arc::new(bits),
            nbits,
            k,
            nparts: 1,
            encoded_parts: Arc::new(Vec::new()),
            loaded: Arc::new(Mutex::new(Vec::new())),
            parts_once: Arc::new(Vec::new()),
        })
    }

    /// Drop live partition bitsets after encode so a miss loads **one** part.
    pub fn freeze_partitions(&mut self) {
        if self.nparts <= 1 {
            return;
        }
        let g = self.loaded.lock().unwrap_or_else(|e| e.into_inner());
        if self.encoded_parts.len() != self.nparts as usize {
            let mut encoded = Vec::with_capacity(self.nparts as usize);
            for p in g.iter() {
                encoded.push(p.as_ref().map_or_else(Vec::new, |x| x.encode()));
            }
            drop(g);
            self.encoded_parts = Arc::new(encoded);
        } else {
            drop(g);
        }
        self.loaded = Arc::new(Mutex::new(Vec::new()));
        self.parts_once = Arc::new((0..self.nparts as usize).map(|_| OnceLock::new()).collect());
        self.bits = Arc::new(Vec::new());
        self.nbits = 0;
        self.k = 0;
    }

    fn decode_mono(buf: &[u8]) -> Result<Self, String> {
        if buf.is_empty() {
            return Ok(Self::always_true());
        }
        if buf.len() < 12 {
            return Err(format!("bloom too short: {} bytes", buf.len()));
        }
        let nbits = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
        let k = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
        let nbytes = u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]);
        let nbytes_us = nbytes as usize;
        if 12 + nbytes_us > buf.len() {
            return Err("bloom bits truncated".into());
        }
        if nbits == 0 || k == 0 || nbytes == 0 {
            return Ok(Self::always_true());
        }
        let residual = (buf.len() - 12) as u64;
        if !bloom_header_ok(nbits, k, nbytes, residual) {
            return Err(format!(
                "bloom header invalid: nbits {nbits} k {k} nbytes {nbytes} (max k {MAX_K})"
            ));
        }
        let bits = buf[12..12 + nbytes_us].to_vec();
        Ok(Self {
            bits: Arc::new(bits),
            nbits,
            k,
            nparts: 1,
            encoded_parts: Arc::new(Vec::new()),
            loaded: Arc::new(Mutex::new(Vec::new())),
            parts_once: Arc::new(Vec::new()),
        })
    }

    fn decode_partitioned(body: &[u8], nparts: u32) -> Result<Self, String> {
        if nparts <= 1 || nparts > 64 {
            return Err(format!("partitioned bloom nparts {nparts}"));
        }
        let mut encoded_parts = Vec::with_capacity(nparts as usize);
        let mut pos = 0usize;
        for _ in 0..nparts {
            if pos + 4 > body.len() {
                return Err("partitioned bloom truncated".into());
            }
            let n = u32::from_le_bytes(body[pos..pos + 4].try_into().unwrap()) as usize;
            pos += 4;
            if pos + n > body.len() {
                return Err("partitioned bloom part truncated".into());
            }
            encoded_parts.push(body[pos..pos + n].to_vec());
            pos += n;
        }
        Ok(Self {
            bits: Arc::new(Vec::new()),
            nbits: 0,
            k: 0,
            nparts,
            encoded_parts: Arc::new(encoded_parts),
            loaded: Arc::new(Mutex::new(vec![None; nparts as usize])),
            parts_once: Arc::new((0..nparts as usize).map(|_| OnceLock::new()).collect()),
        })
    }

    /// Number of bits.
    #[must_use]
    pub fn bit_count(&self) -> u32 {
        self.nbits
    }

    /// Probe count.
    #[must_use]
    pub fn hash_count(&self) -> u32 {
        self.k
    }
}

/// `bit` is always `< nbits ≤ u32::MAX` from the modulo above.
///
/// RFC-0030 P2: written as an explicit bound + cast (identical to
/// `usize::try_from(bit).unwrap_or(0)` for every `bit`, on 32- and 64-bit)
/// so the Aeneas extract of this file is axiom-free on the T1 path
/// (`try_from`/`unwrap_or` extract as axioms otherwise; see
/// `formal/aeneas/EXTRACT.md`).
pub fn bit_index(bit: u64) -> usize {
    if bit > u64::from(u32::MAX) {
        0
    } else {
        #[allow(clippy::cast_possible_truncation)]
        {
            (bit as u32) as usize
        }
    }
}

/// Kirsch–Mitzenmacher probe. `nbits` is the active filter width (`> 0`).
pub fn probe_bit(h1: u64, h2: u64, i: u32, nbits: u64) -> u64 {
    h1.wrapping_add(u64::from(i).wrapping_mul(h2)) % nbits
}

pub fn set_bit(bits: &mut [u8], i: usize) {
    bits[i / 8] |= 1 << (i % 8);
}

pub fn test_bit(bits: &[u8], i: usize) -> bool {
    (bits[i / 8] & (1 << (i % 8))) != 0
}

/// FNV-1a 64 + mix for a second independent hash.
pub fn hash_pair(key: &[u8]) -> (u64, u64) {
    let h1 = fnv1a64(key);
    // Second hash must be non-zero for double hashing.
    let mut h2 = fnv1a64_seed(key, 0x9e37_79b9_7f4a_7c15);
    if h2 == 0 {
        h2 = 0x9e37_79b9_7f4a_7c15;
    }
    (h1, h2)
}

fn fnv1a64(data: &[u8]) -> u64 {
    fnv1a64_seed(data, 0xcbf2_9ce4_8422_2325)
}

/// FNV-1a 64 exposed for independent second digests (remote segment
/// naming — see `RemoteTier::segment_name`).
pub(crate) fn fnv1a64_pub(data: &[u8]) -> u64 {
    fnv1a64(data)
}

fn fnv1a64_seed(data: &[u8], seed: u64) -> u64 {
    let mut hash = seed;
    for b in data {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

/// T1 teeth mutant (hypothetical — no known occurrence in history): query
/// probes `k + 1` bits while the writer set `k`. Any extra probe landing on a
/// clear bit is a false negative. Exists so the T1 tests/proofs can be shown
/// to bite; see `docs/formal/bloom-filter-theorems.md`.
#[must_use]
pub fn may_contain_mut_extra_probe(f: &BloomFilter, key: &[u8]) -> bool {
    if !f.is_active() {
        return true;
    }
    let (h1, h2) = hash_pair(key);
    let nbits = u64::from(f.nbits);
    let mut i = 0u32;
    while i <= f.k {
        if !test_bit(&f.bits, bit_index(probe_bit(h1, h2, i, nbits))) {
            return false;
        }
        i += 1;
    }
    true
}

/// T1 teeth mutant (hypothetical): query perturbs the second hash so probe
/// indices diverge from the ones `insert` set. Exists so the T1 tests/proofs
/// can be shown to bite; see `docs/formal/bloom-filter-theorems.md`.
#[must_use]
pub fn may_contain_mut_hash_mismatch(f: &BloomFilter, key: &[u8]) -> bool {
    if !f.is_active() {
        return true;
    }
    let (h1, h2) = hash_pair(key);
    let h2 = h2 ^ 1;
    let nbits = u64::from(f.nbits);
    let mut i = 0u32;
    while i < f.k {
        if !test_bit(&f.bits, bit_index(probe_bit(h1, h2, i, nbits))) {
            return false;
        }
        i += 1;
    }
    true
}

/// Kani proof harnesses (T1–T4) — compile only under `cargo kani`.
/// Bounds are documented in `docs/formal/bloom-filter-theorems.md`.
#[cfg(kani)]
mod kani_proofs {
    use super::*;

    /// T3 (residual ≤ 8, k ≤ 8): every header in that F166-accepting slice
    /// decodes without panicking, and querying any 4-byte key never panics.
    /// The unbounded `k ≤ MAX_K` half is carried by the Aeneas extract of
    /// this exact body (`scripts/aeneas_bloom.sh` → `BloomKernel.lean`,
    /// `with_capacity`/`decode`/`MAX_K` defs sorry-free); the former Verus
    /// twin `verus/bloom_filter.rs` was deleted 2026-09-09 (it re-proved a
    /// `Vec<bool>`/`u64` model, not this rustc body).
    #[kani::proof]
    #[kani::unwind(24)]
    fn decode_header_ok_yields_safe_filter() {
        let nbits: u32 = kani::any();
        let k: u32 = kani::any();
        let nbytes: u32 = kani::any();
        let payload_len: u32 = kani::any();
        kani::assume(payload_len <= 8);
        kani::assume(nbytes >= 1 && nbytes <= payload_len);
        kani::assume(u64::from(nbytes) >= u64::from(nbits).div_ceil(8));
        kani::assume((1..=8).contains(&k));
        let mut buf: Vec<u8> = Vec::with_capacity(12 + payload_len as usize);
        buf.extend_from_slice(&nbits.to_le_bytes());
        buf.extend_from_slice(&k.to_le_bytes());
        buf.extend_from_slice(&nbytes.to_le_bytes());
        let payload: [u8; 8] = kani::any();
        buf.extend_from_slice(&payload[..payload_len as usize]);
        let f = BloomFilter::decode(&buf).expect("header-ok buffer must decode");
        let key: [u8; 4] = kani::any();
        let _ = f.may_contain(&key);
    }

    /// T1 (one symbolic 2-byte key, tiny decoded filter: 64 bits, k=2):
    /// insert then `may_contain`. Avoids `with_capacity` (symbolic-size
    /// `vec!` + the sizing arithmetic made CBMC stall).
    #[kani::proof]
    #[kani::unwind(8)]
    fn insert_then_may_contain_all_keys() {
        let key: [u8; 2] = kani::any();
        let mut buf = [0u8; 20];
        buf[0..4].copy_from_slice(&64u32.to_le_bytes());
        buf[4..8].copy_from_slice(&2u32.to_le_bytes());
        buf[8..12].copy_from_slice(&8u32.to_le_bytes());
        let mut f = BloomFilter::decode(&buf).expect("tiny header must decode");
        f.insert(&key);
        assert!(f.may_contain(&key), "false negative for inserted key");
    }

    /// T2 (same concrete shape): `decode(encode(f))` reproduces the filter
    /// and keeps the membership decision. Not in `kani_bloom.sh` — encode +
    /// decode + `PartialEq` did not finish on this host.
    #[kani::proof]
    #[kani::unwind(16)]
    fn encode_decode_roundtrip_preserves_filter() {
        let key: [u8; 3] = kani::any();
        let len: usize = kani::any();
        kani::assume(len <= 3);
        let mut f = BloomFilter::with_capacity(8, 10);
        f.insert(&key[..len]);
        let g = BloomFilter::decode(&f.encode()).expect("roundtrip must decode");
        assert!(g == f, "roundtrip changed the filter");
        assert!(g.may_contain(&key[..len]), "roundtrip lost a member");
    }

    /// T4: an inactive filter never rejects any key.
    #[kani::proof]
    #[kani::unwind(8)]
    fn inactive_filter_never_rejects() {
        let key: [u8; 4] = kani::any();
        assert!(BloomFilter::always_true().may_contain(&key));
        assert!(BloomFilter::with_capacity(0, DEFAULT_BITS_PER_KEY).may_contain(&key));
        assert!(BloomFilter::with_capacity(10, 0).may_contain(&key));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_false_negatives() {
        let mut f = BloomFilter::with_capacity(100, DEFAULT_BITS_PER_KEY);
        for i in 0..100u32 {
            f.insert(format!("key-{i}").as_bytes());
        }
        for i in 0..100u32 {
            assert!(
                f.may_contain(format!("key-{i}").as_bytes()),
                "false negative for key-{i}"
            );
        }
    }

    #[test]
    fn encode_decode_round_trip() {
        let mut f = BloomFilter::with_capacity(50, DEFAULT_BITS_PER_KEY);
        f.insert(b"alpha");
        f.insert(b"beta");
        let enc = f.encode();
        let g = BloomFilter::decode(&enc).unwrap();
        assert!(g.may_contain(b"alpha"));
        assert!(g.may_contain(b"beta"));
        assert_eq!(f.bit_count(), g.bit_count());
        assert_eq!(f.hash_count(), g.hash_count());
    }

    #[test]
    fn always_true_never_rejects() {
        let f = BloomFilter::always_true();
        assert!(f.may_contain(b"anything"));
        assert!(!f.is_active());
    }

    #[test]
    fn rejects_many_absent_keys() {
        let mut f = BloomFilter::with_capacity(32, DEFAULT_BITS_PER_KEY);
        for i in 0..32u32 {
            f.insert(format!("present-{i}").as_bytes());
        }
        let mut rejects = 0usize;
        for i in 0..200u32 {
            if !f.may_contain(format!("absent-{i}").as_bytes()) {
                rejects += 1;
            }
        }
        // With ~10 bits/key, expect most of 200 random keys rejected.
        assert!(rejects > 100, "expected many rejections, got {rejects}");
    }

    #[test]
    fn partitioned_miss_loads_one_part() {
        let mut f = BloomFilter::with_partitions(16, DEFAULT_BITS_PER_KEY, 4);
        for i in 0..16u32 {
            f.insert(i.to_be_bytes().as_slice());
        }
        let enc = f.encode();
        let g = BloomFilter::decode(&enc).unwrap();
        reset_filter_part_load_count();
        let mut part0 = None;
        for i in 1000u32..1100 {
            let k = i.to_be_bytes();
            let (h1, _) = hash_pair(&k);
            if crate::filter_partition_kernel::filter_partition(h1, 4) == 0 {
                part0 = Some(k);
                let _ = g.may_contain(&k);
                break;
            }
        }
        assert!(part0.is_some());
        assert_eq!(
            filter_part_load_count(),
            1,
            "first miss must decode one partition"
        );
        if let Some(k) = part0 {
            let _ = g.may_contain(&k);
        }
        assert_eq!(
            filter_part_load_count(),
            1,
            "same partition must stay cached"
        );
    }

    /// F166: a corrupt/hostile `k` near `u32::MAX` must fail decode closed —
    /// `may_contain` would otherwise loop billions of times per point lookup.
    #[test]
    fn decode_rejects_hostile_probe_count() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&64u32.to_le_bytes()); // nbits
        buf.extend_from_slice(&u32::MAX.to_le_bytes()); // k (hostile)
        buf.extend_from_slice(&8u32.to_le_bytes()); // nbytes
        buf.extend_from_slice(&[0u8; 8]);
        let err = BloomFilter::decode(&buf).expect_err("must reject hostile k");
        assert!(
            err.contains("invalid") || err.contains("max k"),
            "got {err}"
        );
        // Header kernel AS-IS accepts it (teeth).
        assert!(bloom_header_ok_as_is(64, u32::MAX, 8, 8));
        assert!(!bloom_header_ok(64, u32::MAX, 8, 8));
    }

    /// Catalog three-teeth plant. Direct `decode_rejects_hostile_probe_count` is **not** this tooth.
    #[test]
    fn bloom_header_ok_on_live_decode_is_not_ok() {
        assert!(!bloom_header_ok(64, u32::MAX, 8, 8));
        assert!(
            bloom_header_ok_as_is(64, u32::MAX, 8, 8),
            "AS-IS tooth: no k bound, accepts u32::MAX probes"
        );
        let mut buf = Vec::new();
        buf.extend_from_slice(&64u32.to_le_bytes());
        buf.extend_from_slice(&u32::MAX.to_le_bytes());
        buf.extend_from_slice(&8u32.to_le_bytes());
        buf.extend_from_slice(&[0u8; 8]);
        assert!(
            BloomFilter::decode(&buf).is_err(),
            "live BloomFilter::decode must fail closed on hostile k"
        );
    }

    /// Catalog three-teeth plant. Direct `no_false_negatives` is **not** this tooth.
    #[test]
    fn insert_on_live_sst_is_not_ok() {
        let key = b"rfc0152-bloom-ins";
        let mut real = BloomFilter::with_capacity(8, DEFAULT_BITS_PER_KEY);
        real.insert(key);
        assert!(real.may_contain(key), "REAL insert must set probe bits");
        let mut as_is = BloomFilter::with_capacity(8, DEFAULT_BITS_PER_KEY);
        as_is.insert_as_is(key);
        assert!(
            !as_is.may_contain(key),
            "AS-IS tooth: skipped insert is a false negative"
        );
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("pedra-bloom-ins-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut db = crate::Db::open_with(
            &dir,
            crate::OpenOptions {
                wal_full_fsync: true,
                history: Default::default(),
                wal_recovery: Default::default(),
                sync: true,
                auto_flush_bytes: None,
                auto_compact_sst_count: None,
                auto_compact_sst_bytes: None,
                exclusive: true,
                large_value_threshold: None,
                sst_payload_budget_bytes: None,
            },
        )
        .unwrap();
        db.put(key, b"ok").unwrap();
        db.flush().unwrap();
        assert_eq!(
            db.get(key).as_deref(),
            Some(b"ok".as_ref()),
            "live SST write must bloom.insert the key; AS-IS skip would miss after flush"
        );
        db.close().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Catalog three-teeth plant. Direct `no_false_negatives` is **not** this tooth.
    #[test]
    fn may_contain_on_live_sst_is_not_ok() {
        let mut teeth = false;
        for i in 0..512u16 {
            let key = i.to_le_bytes();
            let mut f = BloomFilter::with_capacity(8, DEFAULT_BITS_PER_KEY);
            f.insert(&key);
            assert!(f.may_contain(&key), "REAL must not false-negative");
            if !f.may_contain_as_is(&key) {
                teeth = true;
                break;
            }
        }
        assert!(
            teeth,
            "AS-IS tooth: extra probe must false-negative at least one inserted key"
        );
        let key = b"rfc0152-bloom-mc";
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("pedra-bloom-mc-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut db = crate::Db::open_with(
            &dir,
            crate::OpenOptions {
                wal_full_fsync: true,
                history: Default::default(),
                wal_recovery: Default::default(),
                sync: true,
                auto_flush_bytes: None,
                auto_compact_sst_count: None,
                auto_compact_sst_bytes: None,
                exclusive: true,
                large_value_threshold: None,
                sst_payload_budget_bytes: None,
            },
        )
        .unwrap();
        db.put(key, b"ok").unwrap();
        db.flush().unwrap();
        assert_eq!(
            db.get(key).as_deref(),
            Some(b"ok".as_ref()),
            "live SST get must honour may_contain of an inserted key"
        );
        db.close().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RFC-0030: `with_capacity` rewrite (no max/clamp/div_ceil) matches
    /// the previous integer form on the writer grid.
    #[test]
    fn with_capacity_matches_legacy_formula() {
        for bits_per_key in 1..=64usize {
            for n_keys in [1usize, 2, 7, 16, 64, 300] {
                let f = BloomFilter::with_capacity(n_keys, bits_per_key);
                let raw = n_keys.saturating_mul(bits_per_key).max(64);
                let nbits = u32::try_from(raw.min(u32::MAX as usize)).unwrap_or(u32::MAX);
                let k = u32::try_from((bits_per_key * 69) / 100)
                    .unwrap_or(1)
                    .clamp(1, 30);
                assert_eq!(f.bit_count(), nbits, "n_keys={n_keys} bpk={bits_per_key}");
                assert_eq!(f.hash_count(), k, "n_keys={n_keys} bpk={bits_per_key}");
                assert_eq!(f.encode().len(), 12 + (nbits as usize).div_ceil(8));
            }
        }
    }

    /// F166 regression: every k produced by `with_capacity` decodes back.
    #[test]
    fn decode_accepts_every_written_probe_count() {
        for bits_per_key in 1..=64usize {
            let f = BloomFilter::with_capacity(16, bits_per_key);
            assert!(f.hash_count() >= 1 && f.hash_count() <= MAX_K);
            let g = BloomFilter::decode(&f.encode()).unwrap();
            assert_eq!(g.hash_count(), f.hash_count());
        }
    }

    /// RFC-0233 P2.1 Monkey: small runs get more bits/key than large ones.
    #[test]
    fn rfc0233_monkey_small_run_gets_more_bits() {
        let small = bits_per_key_for_run(100);
        let mid = bits_per_key_for_run(10_000);
        let large = bits_per_key_for_run(1_000_000);
        assert!(
            small > mid && mid >= large,
            "small={small} mid={mid} large={large}"
        );
        assert_eq!(bits_per_key_for_run_as_is(100), DEFAULT_BITS_PER_KEY);
        assert_eq!(bits_per_key_for_run_as_is(1_000_000), DEFAULT_BITS_PER_KEY);
        let sst = include_str!("sst/table_kernel.rs");
        assert!(
            sst.contains("bits_per_key_for_run"),
            "SST bloom rebuild must use Monkey bits-per-run"
        );
    }
}
