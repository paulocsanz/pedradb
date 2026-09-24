//! WAL staging-buffer decision kernel (RFC-0209 P0.1). Integer
//! arithmetic, no I/O.
//!
//! The async 1-op write path pays one `write()` syscall per operation
//! (`commit_async_one` → `Wal::write_pending_frame` →
//! `WalWriter::write_frame` → `out.write_all`), while the RocksDB default
//! peer (`WriteOptions.sync=false`) memcpys into a user-space
//! `WritableFile` buffer and only hits the kernel when it fills — measured
//! as the worst same-class losses on the board (ycsb_f_mc4 0,2947×,
//! deps_cache_overwrite_mc4 0,3698× min-of-3, `findings/
//! 2026-09-11-gargalos-inventario/`).
//!
//! This kernel owns exactly one decision: whether the staged buffer must
//! flush NOW, given its size and the configured cap. Size-only by design —
//! RocksDB's `WritableFile` has no idle timer, so parity removes the clock
//! entirely (no timer thread, no wait-to-grow: the vetoes of 0180/0190
//! are structural here, the buffer never waits for writers).

#![forbid(unsafe_code)]

/// Default staging cap (64 KiB) — RocksDB `WritableFile` buffer parity.
pub const WAL_BUF_MAX_DEFAULT_BYTES: u64 = 64 * 1024;

/// Whether a staging buffer holding `staged` bytes must flush now.
///
/// `true` once `staged >= max`: the caller appended the incoming frame,
/// then asks. The comparison is on the post-append size, mirroring
/// `WritableFile::Append` (buffer takes the bytes; a full buffer writes).
///
/// Misuse guard: `max == 0` returns `true` for every call — flush every
/// operation — which is byte-for-byte the AS-IS behavior of one write per
/// op (see [`should_flush_as_is`]). A caller that manages to construct a
/// zero cap therefore cannot lose the drain property.
#[must_use]
pub fn should_flush(staged: u64, max: u64) -> bool {
    max == 0 || staged >= max
}

/// Workload switch: a **lone** writer (1c / `commit_async_one`) flushes
/// every frame — Rocks `FlushWAL` per `Write()`. A concurrent/group
/// frame only flushes at the 64 KiB cap. That is how staging stays
/// default-on without the p209b 1c regression (ycsb_f 1.88→1.09).
#[must_use]
pub fn should_flush_for_workload(staged: u64, max: u64, lone_writer: bool) -> bool {
    lone_writer || should_flush(staged, max)
}

/// AS-IS twin of [`should_flush`] — the pre-RFC-0209 engine had no
/// staging: every frame flushed (went straight to the sink) immediately.
#[must_use]
pub fn should_flush_as_is(_staged: u64, _max: u64) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wal_buffer_should_flush_at_threshold() {
        // Below the cap: keep staging.
        assert!(!should_flush(0, 5));
        assert!(!should_flush(4, 5));
        // At or above the cap (post-append size): flush now.
        assert!(should_flush(5, 5));
        assert!(should_flush(6, 5));
        // Real cap: 64 KiB boundary.
        assert!(!should_flush(
            WAL_BUF_MAX_DEFAULT_BYTES - 1,
            WAL_BUF_MAX_DEFAULT_BYTES
        ));
        assert!(should_flush(
            WAL_BUF_MAX_DEFAULT_BYTES,
            WAL_BUF_MAX_DEFAULT_BYTES
        ));
    }

    #[test]
    fn wal_buffer_as_is_always_flushes() {
        for staged in [0u64, 1, 63, 64 * 1024, 1 << 20] {
            for max in [0u64, 1, 4096, WAL_BUF_MAX_DEFAULT_BYTES] {
                assert!(should_flush_as_is(staged, max));
            }
        }
    }

    #[test]
    #[test]
    fn wal_buffer_lone_always_flushes_group_respects_cap() {
        let cap = WAL_BUF_MAX_DEFAULT_BYTES;
        assert!(
            should_flush_for_workload(1, cap, true),
            "1c FlushWAL per Write"
        );
        assert!(
            !should_flush_for_workload(cap - 1, cap, false),
            "group stages below cap"
        );
        assert!(should_flush_for_workload(cap, cap, false));
    }

    #[test]
    fn wal_buffer_zero_max_is_as_is() {
        // Misuse guard: a zero cap degenerates to the AS-IS dente —
        // flush every op, never accumulate.
        for staged in [0u64, 1, 100, 1 << 20] {
            assert_eq!(should_flush(staged, 0), should_flush_as_is(staged, 0));
        }
    }

    #[test]
    fn wal_buffer_flush_is_monotonic_in_staged() {
        for max in [1u64, 100, WAL_BUF_MAX_DEFAULT_BYTES] {
            let mut decided = false;
            for staged in 0..=(max + 2) {
                let now = should_flush(staged, max);
                assert!(decided || !now || staged >= max);
                decided = decided || now;
            }
            assert!(decided, "max={max} must eventually flush");
        }
    }
}
