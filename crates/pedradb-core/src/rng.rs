//! Entropy / RNG seam for election jitter, IDs, and DST.
//!
//! Production: [`SystemRng`] (process-local xorshift seeded from time+counter).
//! Tests / sim: [`SeedRng`] — fully deterministic from a `u64` seed (replayable).

use std::cell::Cell;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Non-cryptographic random source. `Clone` shares state where the impl does.
pub trait Rng: Clone {
    /// Next `u64` in the stream.
    fn next_u64(&self) -> u64;

    /// Uniform in `0..bound` (bound > 0).
    fn gen_range(&self, bound: u64) -> u64 {
        if bound == 0 {
            return 0;
        }
        self.next_u64() % bound
    }
}

/// Process entropy: xorshift64* seeded once, advanced with an atomic.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemRng;

impl Rng for SystemRng {
    fn next_u64(&self) -> u64 {
        static STATE: AtomicU64 = AtomicU64::new(0);
        let mut s = STATE.load(Ordering::Relaxed);
        if s == 0 {
            let seed = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0xA11CE, |d| {
                    #[allow(clippy::cast_possible_truncation)]
                    {
                        // Truncate nanos to u64 entropy (intentional for seeding).
                        d.as_nanos() as u64
                    }
                });
            let seed = seed ^ (seed << 13) ^ 0xDEAD_BEEF_CAFE_BABE;
            let _ = STATE.compare_exchange(0, seed | 1, Ordering::Relaxed, Ordering::Relaxed);
            s = STATE.load(Ordering::Relaxed);
        }
        // xorshift64*
        s ^= s >> 12;
        s ^= s << 25;
        s ^= s >> 27;
        STATE.store(s, Ordering::Relaxed);
        s.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
}

/// Deterministic RNG: same seed ⇒ same stream (DST / election tests).
///
/// Clones share the same `Cell` state so consumers of a shared host advance one stream.
#[derive(Debug, Clone)]
pub struct SeedRng {
    state: Rc<Cell<u64>>,
}

impl SeedRng {
    /// Seed the stream.
    ///
    /// **Important (DST):** consecutive seeds must not collapse. Older code used
    /// `seed | 1`, which mapped every even `n` to the same stream as `n+1` and
    /// halved unique World schedules under `for seed in 1..=N`. We only reject
    /// the xorshift fixed point `0`, via a SplitMix64-style mix so each `u64`
    /// seed yields a distinct initial state (and `0` still works).
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self {
            state: Rc::new(Cell::new(mix_seed(seed))),
        }
    }

    /// Current internal state (for debugging / logging only).
    #[must_use]
    pub fn state(&self) -> u64 {
        self.state.get()
    }
}

/// Bijective-ish seed mix (`SplitMix64` finalizer). Never returns 0.
#[must_use]
pub fn mix_seed(seed: u64) -> u64 {
    let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    if z == 0 {
        0x000A_11CE_5EED_u64
    } else {
        z
    }
}

impl Rng for SeedRng {
    fn next_u64(&self) -> u64 {
        let mut s = self.state.get();
        s ^= s >> 12;
        s ^= s << 25;
        s ^= s >> 27;
        self.state.set(s);
        s.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_rng_replayable() {
        let a = SeedRng::new(42);
        let b = SeedRng::new(42);
        let va: Vec<u64> = (0..16).map(|_| a.next_u64()).collect();
        let vb: Vec<u64> = (0..16).map(|_| b.next_u64()).collect();
        assert_eq!(va, vb);
    }

    #[test]
    fn seed_rng_shared_clone() {
        let a = SeedRng::new(7);
        let b = a.clone();
        let _ = a.next_u64();
        // b continues the same stream (not a fork).
        assert_ne!(b.next_u64(), SeedRng::new(7).next_u64());
    }

    #[test]
    fn consecutive_seeds_do_not_collapse() {
        // Regression: `seed | 1` made SeedRng::new(2) ≡ SeedRng::new(3).
        let a = SeedRng::new(2);
        let b = SeedRng::new(3);
        let va: Vec<u64> = (0..8).map(|_| a.next_u64()).collect();
        let vb: Vec<u64> = (0..8).map(|_| b.next_u64()).collect();
        assert_ne!(va, vb, "even/odd seeds must not share a stream");
        // Many consecutive seeds → all distinct first outputs.
        let mut firsts = std::collections::HashSet::new();
        for s in 1..=64u64 {
            firsts.insert(SeedRng::new(s).next_u64());
        }
        assert_eq!(
            firsts.len(),
            64,
            "expected 64 distinct first draws from seeds 1..=64, got {}",
            firsts.len()
        );
        assert_ne!(mix_seed(0), 0);
        assert_ne!(mix_seed(0), mix_seed(1));
    }
}
