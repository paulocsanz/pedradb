//! Abstract crash semantics of the [`crate::env`] seam (RFC-0166 P1.1).
//!
//! The live seams are the production `Env`/`EnvFile` (`env.rs`: `sync_data`
//! is the barrier) and the sim `RecordingEnv` (buffered writes, honest sync
//! promotes, `SyncPolicy::Lying` returns Ok without promoting). This kernel
//! is the pure geometry every crash story must respect:
//!
//! - a byte log has `written` (appended, maybe buffered) and `synced` (the
//!   durable barrier floor);
//! - a crash may keep ANY prefix `cut` with `synced <= cut <= written` —
//!   the floor is the honest-sync barrier (synced bytes never vanish), the
//!   ceiling is "no byte is ever invented" (a torn write may keep a
//!   PREFIX of the unsynced tail, never more than was written);
//! - honest sync promotes all pending (`synced == written`); a lying sync
//!   returns Ok and promotes nothing (RFC-0078), via the proved
//!   [`crate::group_commit_kernel::fsync_promotes_pending`].
//!
//! AS-IS mutants drop the floor (an unsynced tail pretends barrier
//! durability) and pretend lying sync promotes. Teeth witnesses pin both
//! holes. Verus twin: `crates/pedradb-core/verus/env_crash.rs`
//! (`scripts/verus_env_crash.sh`).

#![forbid(unsafe_code)]

use crate::group_commit_kernel::fsync_promotes_pending;

/// Model of the sync honesty at the seam: honest OS/env vs `SyncPolicy::Lying`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncHonesty {
    /// `sync_data` Ok ⇒ pending promoted (the real barrier).
    Honest,
    /// `SyncPolicy::Lying`: Ok without promoting (RFC-0078).
    Lying,
}

/// Byte-log geometry: appended length and the durable barrier floor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CrashModel {
    /// Bytes appended (logical length, possibly buffered).
    pub written: u64,
    /// Bytes the last honest barrier made crash-proof (`synced <= written`).
    pub synced: u64,
}

impl CrashModel {
    /// Well-formed log: barrier never past the appended length.
    #[must_use]
    pub fn of(written: u64, synced: u64) -> Self {
        Self {
            written,
            synced: synced.min(written),
        }
    }
}

/// Append `n` bytes: logical length grows; the barrier does not move.
#[must_use]
pub fn append(m: CrashModel, n: u64) -> CrashModel {
    CrashModel {
        written: m.written + n,
        synced: m.synced,
    }
}

/// Sync per honesty: honest promotes every pending byte (barrier becomes
/// the full length); lying returns Ok and changes nothing.
#[must_use]
pub fn sync(m: CrashModel, honesty: SyncHonesty) -> CrashModel {
    if fsync_promotes_pending(honesty == SyncHonesty::Honest) {
        CrashModel {
            written: m.written,
            synced: m.written,
        }
    } else {
        m
    }
}

/// A crash outcome is legal iff the surviving prefix sits between the
/// barrier floor and the written ceiling — torn tails may keep a prefix,
/// synced bytes never vanish, no byte is invented.
#[must_use]
pub fn crash_legal(m: CrashModel, cut: u64) -> bool {
    m.synced <= cut && cut <= m.written
}

/// Corollary (floor): a legal crash never loses a synced byte.
#[must_use]
pub fn barrier_floor_holds(m: CrashModel, cut: u64) -> bool {
    !crash_legal(m, cut) || cut >= m.synced
}

/// Corollary (ceiling): a legal crash never survives past `written` —
/// recovery can never observe a byte the writer never appended.
#[must_use]
pub fn no_invented_bytes_holds(m: CrashModel, cut: u64) -> bool {
    !crash_legal(m, cut) || cut <= m.written
}

/// Honest sync is a real barrier: after it, every legal crash keeps the
/// whole log.
#[must_use]
pub fn honest_sync_protects_all(m: CrashModel, cut: u64) -> bool {
    let s = sync(m, SyncHonesty::Honest);
    !crash_legal(s, cut) || cut == s.written
}

/// AS-IS hole 1 (torn floor): any cut up to `written` is "legal" — the
/// barrier floor is ignored, so a crash may eat bytes the app was told
/// were synced.
#[must_use]
pub fn crash_legal_as_is(m: CrashModel, cut: u64) -> bool {
    cut <= m.written
}

/// AS-IS hole 2: a lying sync pretends it promoted (RFC-0078 as-is —
/// `fsync_promotes_pending_as_is`).
#[must_use]
pub fn sync_lying_promotes_as_is(m: CrashModel) -> CrashModel {
    CrashModel {
        written: m.written,
        synced: m.written,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn honest_barrier_then_crash_keeps_everything() {
        let m = append(CrashModel::of(0, 0), 8);
        let s = sync(m, SyncHonesty::Honest);
        assert_eq!(s.synced, 8);
        assert!(crash_legal(s, 8));
        assert!(honest_sync_protects_all(m, 8));
        // The full append survives even the worst legal cut.
        assert!(crash_legal(s, 8) && s.synced == 8);
    }

    #[test]
    fn crash_legal_as_is_does_not_imply_barrier_floor() {
        // synced=5 barrier, crash "legal" per AS-IS at cut=3: bytes the
        // honest barrier promised (5) are gone — the floor theorem fails.
        let m = CrashModel::of(10, 5);
        assert!(crash_legal_as_is(m, 3));
        assert!(!crash_legal(m, 3));
        assert!(barrier_floor_holds(m, 3));
    }

    #[test]
    fn lying_sync_does_not_promote() {
        // RFC-0078: Lying sync returns Ok; the model must not move the
        // barrier, and the crash may drop everything unsynced.
        let m = append(CrashModel::of(0, 0), 4);
        let s = sync(m, SyncHonesty::Lying);
        assert_eq!(s.synced, 0);
        assert!(crash_legal(s, 0));
        // AS-IS pretends the lying barrier promoted; the model refutes.
        assert_eq!(sync_lying_promotes_as_is(m).synced, 4);
        assert_ne!(s, sync_lying_promotes_as_is(m));
    }

    #[test]
    fn torn_tail_is_prefix_only() {
        // 10 written, 4 synced: every legal cut is a prefix in [4, 10];
        // 11 (inventing a byte) is illegal even per AS-IS ceiling.
        let m = CrashModel::of(10, 4);
        for cut in 0..=12 {
            assert_eq!(crash_legal(m, cut), (4..=10).contains(&cut));
            assert!(no_invented_bytes_holds(m, cut));
        }
    }
}
