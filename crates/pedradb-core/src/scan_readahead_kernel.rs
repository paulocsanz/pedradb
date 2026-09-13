//! kernel: scan readahead window (RFC-0195 P0.1) — when may a sequential
//! scan's block walk ask the kernel to read ahead
//! (`POSIX_FADV_WILLNEED` through the `SstFileSource`/`Env::advise`
//! seam)?
//!
//! Today every SST fd opens `FADV_RANDOM` (Rocks-parity
//! `set_advise_random_on_open` — right for point gets; the v69
//! lookup_100 0.87× lesson) and every block read is one 4 KiB `pread`,
//! so a bounded-cache scan pays a cold random-read latency per block by
//! construction. The cut: a `WILLNEED` window ahead of the scan cursor,
//! **iff** (a) the store is in bounded-cache mode (SST bytes above the
//! WARM cap — the same predicate family as RFC-0194; fitting/hot stores
//! never issue) and (b) the block about to be read is file-adjacent to
//! the next one (the walk is provably sequential, run ≥ 2). The
//! unconditional version is the v69/Fire-118 regression class and is
//! vetoed. Pure integer units; the scan walk that CALLS this kernel
//! lives in `db.rs`.

#![forbid(unsafe_code)]

/// Window ceiling: 64 blocks × 4 KiB — one advise per ~64 preads. A
/// larger window in a 4 GiB store evicts the warm set it is trying to
/// protect (the Fire-118 lesson applied to the read side).
pub const SCAN_READAHEAD_CAP_BYTES: u64 = 256 * 1024;

/// Enough handle pairs to fill the cap at 4 KiB blocks (64) plus the
/// anchor pair — the production collector never walks past this.
const MAX_WINDOW_PAIRS: usize = 66;

/// Verdict of the scan-readahead policy for one block load. `len == 0`
/// means "do not advise" (every keep case).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ScanReadaheadWindow {
    /// File offset of the first block in the window.
    pub offset: u64,
    /// Window bytes to `WILLNEED` (0 = do not advise). Whole blocks
    /// only — the cap never splits a block.
    pub len: u64,
    /// Blocks covered by the window.
    pub blocks: u32,
}

impl ScanReadaheadWindow {
    /// The universal "no readahead" verdict.
    #[must_use]
    pub const NONE: ScanReadaheadWindow = ScanReadaheadWindow {
        offset: 0,
        len: 0,
        blocks: 0,
    };
}

/// RFC-0195 P0.1: the window as a total function.
///
/// `blocks[i] = (offset, len)` of file block `i` in walk order; `at` is
/// the block about to be read. Fires iff `bounded_cache` (SST bytes >
/// warm cap, decided upstream) and `blocks[at + 1]` starts exactly where
/// `blocks[at]` ends — the walk is sequential, so the blocks after the
/// cursor are the ones it is about to touch. The window then covers the
/// adjacent run from `at + 1`, capped at [`SCAN_READAHEAD_CAP_BYTES`]
/// whole blocks.
#[must_use]
pub fn scan_readahead_window(
    blocks: &[(u64, u64)],
    at: usize,
    bounded_cache: bool,
) -> ScanReadaheadWindow {
    if !bounded_cache {
        return ScanReadaheadWindow::NONE;
    }
    let Some(&(cur_off, cur_len)) = blocks.get(at) else {
        return ScanReadaheadWindow::NONE;
    };
    let Some(&(next_off, _)) = blocks.get(at + 1) else {
        return ScanReadaheadWindow::NONE;
    };
    // Run ≥ 2: the next block must continue the current one on disk —
    // a seek-heavy walk (probe jumps) never proves sequentiality.
    if next_off != cur_off.saturating_add(cur_len) {
        return ScanReadaheadWindow::NONE;
    }
    let mut window = ScanReadaheadWindow {
        offset: next_off,
        len: 0,
        blocks: 0,
    };
    let mut expected_off = next_off;
    let mut k = at + 1;
    while window.len < SCAN_READAHEAD_CAP_BYTES {
        let Some(&(off, len)) = blocks.get(k) else {
            break;
        };
        if off != expected_off {
            break;
        }
        if window.len.saturating_add(len) > SCAN_READAHEAD_CAP_BYTES {
            break;
        }
        window.len += len;
        window.blocks += 1;
        expected_off = off.saturating_add(len);
        k += 1;
    }
    if window.blocks == 0 {
        return ScanReadaheadWindow::NONE;
    }
    window
}

/// AS-IS twin: today's engine never reads ahead (every block is one
/// cold 4 KiB pread behind `FADV_RANDOM` fds).
#[must_use]
pub fn scan_readahead_window_as_is(
    _blocks: &[(u64, u64)],
    _at: usize,
    _bounded_cache: bool,
) -> ScanReadaheadWindow {
    ScanReadaheadWindow::NONE
}

/// Bounded-cache predicate (RFC-0195 P0.2): live SST bytes above the warm
/// cap — the RFC-0194 `KeepHot` boundary. Fitting/hot stores never read
/// ahead (the Fire-118 lesson, read side).
#[must_use]
pub const fn scan_readahead_bounded(sst_bytes: u64, warm_cap: u64) -> bool {
    sst_bytes > warm_cap
}

/// How many handle pairs the production collector needs starting at the
/// anchor block (enough to fill the cap at 4 KiB blocks).
#[must_use]
pub const fn scan_readahead_pair_budget() -> usize {
    MAX_WINDOW_PAIRS
}

#[cfg(test)]
mod tests {
    use super::*;

    const KIB: u64 = 1024;

    fn adjacent_run(n: usize, block: u64) -> Vec<(u64, u64)> {
        (0..n).map(|i| (i as u64 * block, block)).collect()
    }

    /// Non-bounded (fitting/hot) stores never read ahead — the
    /// v69/Fire-118 lesson: readahead on a fitting store or a point-get
    /// pattern is pure eviction pressure.
    #[test]
    fn fitting_store_never_reads_ahead() {
        let blocks = adjacent_run(8, 4 * KIB);
        assert_eq!(
            scan_readahead_window(&blocks, 2, false),
            ScanReadaheadWindow::NONE
        );
    }

    /// Sequential walk in a bounded store: the window starts at the
    /// block after the anchor and covers the rest of the adjacent run.
    #[test]
    fn adjacent_run_extends_window() {
        let blocks = adjacent_run(5, 4 * KIB);
        let w = scan_readahead_window(&blocks, 1, true);
        assert_eq!(w.offset, 2 * 4 * KIB, "window starts at block at+1");
        assert_eq!(w.len, 3 * 4 * KIB, "covers blocks 2..=4");
        assert_eq!(w.blocks, 3);
    }

    /// Run < 2 (next block not file-adjacent — a probe jump) never
    /// issues, even in a bounded store.
    #[test]
    fn non_adjacent_run_is_zero() {
        let mut blocks = adjacent_run(4, 4 * KIB);
        // Gap between block 1 and block 2 (the tail keeps its own run).
        blocks[2].0 += 128;
        blocks[3].0 += 128;
        assert_eq!(
            scan_readahead_window(&blocks, 1, true),
            ScanReadaheadWindow::NONE,
            "anchor at 1: block 2 no longer continues block 1"
        );
        // Anchor inside the tail's own adjacent run still fires.
        let w = scan_readahead_window(&blocks, 2, true);
        assert_eq!(w.blocks, 1, "blocks 2..3 remain adjacent");
    }

    /// The window caps at 256 KiB of whole blocks.
    #[test]
    fn window_caps_at_256_kib() {
        let blocks = adjacent_run(200, 4 * KIB);
        let w = scan_readahead_window(&blocks, 0, true);
        assert_eq!(w.len, SCAN_READAHEAD_CAP_BYTES);
        assert_eq!(w.blocks, 64, "256 KiB / 4 KiB blocks");
        assert_eq!(w.offset, 4 * KIB, "starts after the anchor");
    }

    /// Larger blocks cap on whole blocks too (never split a block).
    #[test]
    fn cap_never_splits_a_block() {
        let blocks = adjacent_run(10, 100 * KIB);
        let w = scan_readahead_window(&blocks, 0, true);
        assert_eq!(
            w.len,
            200 * KIB,
            "two 100 KiB blocks fit, third would cross the cap"
        );
        assert_eq!(w.blocks, 2);
    }

    /// Anchor at the tail (no next block) and out-of-range anchors: zero.
    #[test]
    fn tail_and_oob_anchors_are_zero() {
        let blocks = adjacent_run(3, 4 * KIB);
        assert_eq!(
            scan_readahead_window(&blocks, 2, true),
            ScanReadaheadWindow::NONE
        );
        assert_eq!(
            scan_readahead_window(&blocks, 7, true),
            ScanReadaheadWindow::NONE
        );
    }

    /// AS-IS twin: today's engine never reads ahead, whatever the shape.
    #[test]
    fn as_is_twin_is_always_zero() {
        let blocks = adjacent_run(200, 4 * KIB);
        assert_eq!(
            scan_readahead_window_as_is(&blocks, 0, true),
            ScanReadaheadWindow::NONE
        );
    }

    /// The production pair budget is enough to fill the cap at 4 KiB
    /// blocks (64 window blocks + the anchor).
    #[test]
    fn pair_budget_fills_the_cap() {
        assert!(scan_readahead_pair_budget() >= 65);
        let n = scan_readahead_pair_budget();
        let blocks = adjacent_run(n, 4 * KIB);
        let w = scan_readahead_window(&blocks, 0, true);
        assert_eq!(w.len, SCAN_READAHEAD_CAP_BYTES, "budget reaches the cap");
    }

    /// The bounded boundary is the 0194 `KeepHot` line: bytes at the cap
    /// are hot (no readahead), one byte above is bounded.
    #[test]
    fn bounded_boundary_is_the_0194_warm_cap() {
        let cap = 3 * 1024 * 1024 * 1024u64;
        assert!(!scan_readahead_bounded(cap, cap), "at the cap: hot");
        assert!(scan_readahead_bounded(cap + 1, cap), "above: bounded");
        assert!(!scan_readahead_bounded(0, cap), "empty store: hot");
    }
}
