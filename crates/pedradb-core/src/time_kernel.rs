//! Logical / wall clock seam for leases, TTL, and DST (non-determinism control).
//!
//! Production: [`SystemClock`]. Tests / sim: [`ManualClock`] (advance without wall time).
//! DST harnesses later plug the same trait without pulling `determinismo` into the engine.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// Source of time for the stack (DCS leases, future store timeouts, DST).
///
/// `Clone` so components can hold a shared clock (manual clocks share advance state via `Rc`).
pub trait Clock: Clone {
    /// Current instant (monotonic for logical clocks).
    fn now(&self) -> Instant;
}

/// Wall-clock [`Instant::now`].
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// Deterministic clock: start frozen, advance explicitly.
///
/// Clones share the same offset so `advance` is visible everywhere.
#[derive(Debug, Clone)]
pub struct ManualClock {
    base: Instant,
    /// Nanoseconds added to `base`.
    offset_ns: Rc<Cell<u64>>,
}

impl ManualClock {
    /// Frozen at construction time (`offset = 0`).
    #[must_use]
    pub fn new() -> Self {
        Self {
            base: Instant::now(),
            offset_ns: Rc::new(Cell::new(0)),
        }
    }

    /// Start with an explicit offset (tests).
    #[must_use]
    pub fn with_offset(d: Duration) -> Self {
        let c = Self::new();
        c.advance(d);
        c
    }

    /// Advance logical time by `d` (shared across clones).
    pub fn advance(&self, d: Duration) {
        let add = u64::try_from(d.as_nanos()).unwrap_or(u64::MAX);
        let cur = self.offset_ns.get();
        self.offset_ns.set(cur.saturating_add(add));
    }

    /// Nanoseconds since construction epoch (for logging / seeds).
    #[must_use]
    pub fn elapsed_ns(&self) -> u64 {
        self.offset_ns.get()
    }
}

impl Default for ManualClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Instant {
        self.base + Duration::from_nanos(self.offset_ns.get())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_clock_shared_advance() {
        let c = ManualClock::new();
        let c2 = c.clone();
        let t0 = c.now();
        c.advance(Duration::from_millis(50));
        assert!(c2.now() >= t0 + Duration::from_millis(49));
        assert_eq!(c.elapsed_ns(), c2.elapsed_ns());
    }
}
