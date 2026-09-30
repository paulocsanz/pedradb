//! RFC-0305 hydrate writer-phase diagnostics (`PEDRA_HYDRATE_DIAG=1`).
//!
//! The 100M hydrate gap vs the Rocks peer is writer-side and
//! machine-dependent; these counters split the latched-bulk submit path
//! into its phases so a bench run can attribute the remaining budget:
//! debt waits (parked queue backpressure), Db write-lock acquisition,
//! the apply body, and the superversion publish. Zero cost when the env
//! is unset (one relaxed atomic load per batch; the phase timing is
//! `Option<Instant>` and compiles to nothing on the cold path).
//!
//! Read the line as: `HYDRATEDIAG batches=… entries=… debt_ms=…
//! lock_ms=… apply_ms=… publish_ms=…` — printed by the compat layer at
//! DB drop.

#![forbid(unsafe_code)]

use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

/// Phase slots (ns accumulators). Indices are part of the diag contract.
pub const LB_DEBT: usize = 0;
pub const LB_LOCK: usize = 1;
pub const LB_APPLY: usize = 2;
pub const LB_PUBLISH: usize = 3;
pub const LB_BATCHES: usize = 4;
pub const LB_ENTRIES: usize = 5;

static COUNTERS: [AtomicU64; 6] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];

static ENABLED: AtomicU64 = AtomicU64::new(0);

/// Whether phase accounting is on (`PEDRA_HYDRATE_DIAG=1`). Latched to a
/// nonzero value after the first lookup so the env scan amortizes to zero.
#[must_use]
pub fn latched_bulk_diag_enabled() -> bool {
    match ENABLED.load(Ordering::Relaxed) {
        0 => {}
        1 => return false,
        2 => return true,
        _ => return false,
    }
    let on = std::env::var_os("PEDRA_HYDRATE_DIAG").is_some_and(|v| v == "1");
    ENABLED.store(if on { 2 } else { 1 }, Ordering::Relaxed);
    on
}

/// Add `ns` to phase `slot` (no-op semantics when diag is off — callers
/// gate the timing themselves; this only accumulates).
pub fn latched_bulk_add(slot: usize, ns: u64) {
    if slot < LB_BATCHES {
        COUNTERS[slot].fetch_add(ns, Ordering::Relaxed);
    }
}

/// Count one completed latched batch of `entries` ops.
pub fn latched_bulk_count(batches: u64, entries: u64) {
    COUNTERS[LB_BATCHES].fetch_add(batches, Ordering::Relaxed);
    COUNTERS[LB_ENTRIES].fetch_add(entries, Ordering::Relaxed);
}

/// One-line dump of the accumulated phases (empty when nothing was counted).
#[must_use]
pub fn latched_bulk_diag_line() -> String {
    let debt = COUNTERS[LB_DEBT].load(Ordering::Relaxed);
    let lock = COUNTERS[LB_LOCK].load(Ordering::Relaxed);
    let apply = COUNTERS[LB_APPLY].load(Ordering::Relaxed);
    let publish = COUNTERS[LB_PUBLISH].load(Ordering::Relaxed);
    let batches = COUNTERS[LB_BATCHES].load(Ordering::Relaxed);
    let entries = COUNTERS[LB_ENTRIES].load(Ordering::Relaxed);
    let ms = |ns: u64| ns as f64 / 1e6;
    format!(
        "HYDRATEDIAG batches={batches} entries={entries} debt_ms={:.1} lock_ms={:.1} apply_ms={:.1} publish_ms={:.1}",
        ms(debt),
        ms(lock),
        ms(apply),
        ms(publish)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_accumulate_and_dump_lists_phases() {
        latched_bulk_add(LB_APPLY, 1_000_000);
        latched_bulk_add(LB_LOCK, 2_000_000);
        latched_bulk_count(1, 1024);
        let line = latched_bulk_diag_line();
        assert!(line.contains("batches=1"), "line: {line}");
        assert!(line.contains("entries=1024"), "line: {line}");
        assert!(line.contains("apply_ms=1.0"), "line: {line}");
        assert!(line.contains("lock_ms=2.0"), "line: {line}");
    }
}
