//! RFC-0236 partitioned Bloom: which filter partition a point miss loads.
//! Integer remainder; no I/O, no env pin, no `PEDRA_FILTER_PARTS`.
//!
//! **Term:** this file is what `rustc` links. Aeneas extracts that body
//! (`scripts/aeneas_filter_partition.sh`).
//!
//!   ./scripts/aeneas_filter_partition.sh --required
//!
//! AS-IS always returns partition 0 (one unpartitioned filter — the
//! pre-0236 shape that loaded the whole Bloom on every miss).

#![forbid(unsafe_code)]

/// How many filter partitions a run of `n_keys` gets. Small runs stay
/// one filter; otherwise four ~equal parts (Fjall 3 / Rocks partitioned
/// filter). Not an env pin.
#[must_use]
pub fn filter_nparts(n_keys: u64) -> u32 {
    if n_keys < 4 {
        1
    } else {
        4
    }
}

/// AS-IS: every run is one filter.
#[must_use]
pub fn filter_nparts_as_is(_n_keys: u64) -> u32 {
    1
}

/// Partition index in `0..nparts` from a key hash (`h1`). `nparts <= 1`
/// collapses to 0.
#[must_use]
pub fn filter_partition(h1: u64, nparts: u32) -> u32 {
    if nparts <= 1 {
        return 0;
    }
    (h1 % u64::from(nparts)) as u32
}

/// AS-IS: always partition 0 (monolithic Bloom).
#[must_use]
pub fn filter_partition_as_is(_h1: u64, _nparts: u32) -> u32 {
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_partition_on_live_scan_window_is_not_ok() {
        assert_eq!(filter_partition(5, 4), 1);
        assert_eq!(
            filter_partition_as_is(5, 4),
            0,
            "AS-IS dente: every key still partition 0"
        );
    }

    #[test]
    fn filter_nparts_small_run_is_one() {
        assert_eq!(filter_nparts(0), 1);
        assert_eq!(filter_nparts(3), 1);
        assert_eq!(filter_nparts(4), 4);
        assert_eq!(filter_nparts_as_is(4), 1);
    }

    #[test]
    fn filter_partition_collapses_when_one_part() {
        assert_eq!(filter_partition(99, 1), 0);
        assert_eq!(filter_partition(99, 0), 0);
    }
}
