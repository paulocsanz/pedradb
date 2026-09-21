//! Bundle of non-determinism / I/O seams for production vs DST.
//!
//! Keep **determinismo** harnesses out-of-tree: they implement these traits
//! (or wrap [`StdHost`] / sim types) without living inside the engine crate.
//!
//! | Seam | Trait | Production | Test / DST |
//! |------|-------|------------|------------|
//! | Disk | [`Env`](crate::env::Env) | [`StdEnv`](crate::env::StdEnv) | `FailingEnv`, `RecordingEnv` |
//! | Time | [`Clock`](crate::time::Clock) | [`SystemClock`](crate::time::SystemClock) | [`ManualClock`](crate::time::ManualClock) |
//! | Entropy | [`Rng`](crate::rng::Rng) | [`SystemRng`](crate::rng::SystemRng) | [`SeedRng`](crate::rng::SeedRng) |

use crate::env::{Env, StdEnv};
use crate::rng::{Rng, SeedRng, SystemRng};
use crate::time::{Clock, ManualClock, SystemClock};

/// Full host: filesystem + clock + entropy.
///
/// Prefer threading concrete types (`StdHost`, `DetHost`) at test boundaries
/// rather than `dyn` — same monomorphization style as `Db<E: Env>`.
pub trait Host: Clone {
    /// Filesystem implementation.
    type Env: Env;
    /// Clock implementation.
    type Clock: Clock;
    /// RNG implementation.
    type Rng: Rng;

    /// Disk / dir ops.
    fn env(&self) -> &Self::Env;
    /// Time source.
    fn clock(&self) -> &Self::Clock;
    /// Entropy source.
    fn rng(&self) -> &Self::Rng;
}

/// Production host: real disk, wall clock, process RNG.
#[derive(Debug, Default, Clone, Copy)]
pub struct StdHost {
    env: StdEnv,
    clock: SystemClock,
    rng: SystemRng,
}

impl StdHost {
    /// Construct production host.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl Host for StdHost {
    type Env = StdEnv;
    type Clock = SystemClock;
    type Rng = SystemRng;

    fn env(&self) -> &StdEnv {
        &self.env
    }

    fn clock(&self) -> &SystemClock {
        &self.clock
    }

    fn rng(&self) -> &SystemRng {
        &self.rng
    }
}

/// Deterministic host for in-process DST (manual clock + seed RNG + pluggable env).
///
/// Disk is still whatever `E` is — use `FailingEnv` / `RecordingEnv` from `pedradb-sim`
/// without coupling this crate to sim.
#[derive(Debug, Clone)]
pub struct DetHost<E: Env> {
    env: E,
    clock: ManualClock,
    rng: SeedRng,
}

impl<E: Env> DetHost<E> {
    /// Build with explicit env, clock, and seed.
    #[must_use]
    pub fn new(env: E, clock: ManualClock, seed: u64) -> Self {
        Self {
            env,
            clock,
            rng: SeedRng::new(seed),
        }
    }

    /// Env + seed; clock starts at zero offset.
    #[must_use]
    pub fn with_seed(env: E, seed: u64) -> Self {
        Self::new(env, ManualClock::new(), seed)
    }

    /// Mutable access to advance time in tests.
    pub fn clock_mut(&self) -> &ManualClock {
        &self.clock
    }

    /// Shared seed RNG (for logging state).
    #[must_use]
    pub fn seed_rng(&self) -> &SeedRng {
        &self.rng
    }
}

impl<E: Env> Host for DetHost<E> {
    type Env = E;
    type Clock = ManualClock;
    type Rng = SeedRng;

    fn env(&self) -> &E {
        &self.env
    }

    fn clock(&self) -> &ManualClock {
        &self.clock
    }

    fn rng(&self) -> &SeedRng {
        &self.rng
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::StdEnv;
    use std::time::Duration;

    #[test]
    fn det_host_seed_stable() {
        let h1 = DetHost::with_seed(StdEnv, 99);
        let h2 = DetHost::with_seed(StdEnv, 99);
        assert_eq!(h1.rng().next_u64(), h2.rng().next_u64());
        h1.clock().advance(Duration::from_secs(1));
        assert!(h1.clock().elapsed_ns() >= 1_000_000_000);
    }
}
