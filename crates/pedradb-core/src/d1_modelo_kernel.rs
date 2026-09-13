//! Named model-level D1 corollary (RFC-0166 P1.3): **put Ok ⇒ survives
//! every torn prefix**.
//!
//! Composes the P1.1 crash geometry ([`crate::env_crash_kernel`]) with the
//! P1.2 inductive invariant ([`crate::wal::wal_state_kernel`]):
//!
//! - [`put_ok`] is the model write path of one put: append → honest sync →
//!   ack. The caller holds `Ok` exactly in the returned state, where the
//!   record sits inside the acked prefix.
//! - [`d1_modelo`] is the named corollary: for every state satisfying
//!   Inv-WAL, every record inside the acked prefix, and every legal torn
//!   prefix `cut`, the cut keeps the record whole (`cut >= rec_end`) — the
//!   put survives every torn write the disk may keep.
//! - [`put_lying_never_acks`]: under `SyncPolicy::Lying` the barrier never
//!   promotes, so the ack is refused and `Ok` is never returned — D1's
//!   premise is false (the guarantee is suspended, not broken).
//!
//! AS-IS mutants ack without the barrier and call torn cuts below the
//! barrier floor "survivable" — both are witnessed to break the corollary.
//!
//! Single artifact (RFC-0171): the rustc body above **is** the proof
//! object — extracted whole-file by Charon+Aeneas (`scripts/aeneas_d1_modelo.sh`
//! → `out/lean/D1ModeloKernel.lean`, sorry-free; theorems
//! `formal/aeneas/lean/D1Modelo.lean`). RFC-0170 P2.3 close citations:
//! `put_ok` refines `prefix_exclusive_end` (torn prefix never splits a
//! record) and the write-ack barrier (`write_ack` family, RFC-0166 P1.4);
//! Inv-WAL via `wal_state_kernel::inv_wal`. The former Verus twin
//! `verus/d1_modelo.rs` (and runner `scripts/verus_d1_modelo.sh`) was
//! deleted 2026-09-09: it re-proved a same-shaped state model, not this
//! rustc body.

#![forbid(unsafe_code)]

use crate::env_crash_kernel::{
    crash_legal, crash_legal_as_is, sync as env_sync, CrashModel, SyncHonesty,
};
use crate::wal::wal_state_kernel::{inv_wal, wal_ack, wal_append, wal_sync, WalState};

/// Model write path of one put: append `rec_len` bytes, honest sync (the
/// product barrier), ack every durable pending byte (group-commit
/// semantics). The caller holds `Ok` in the returned state, where the
/// whole log — the new record included — sits inside the acked prefix.
#[must_use]
pub fn put_ok(s0: WalState, rec_len: u64) -> WalState {
    let s1 = wal_append(s0, rec_len);
    let s2 = wal_sync(s1, SyncHonesty::Honest);
    wal_ack(s2, s2.synced - s2.acked)
}

/// Named corollary D1-model: the record inside the acked prefix of an
/// Inv-WAL state survives every legal torn prefix (the cut keeps the
/// record whole).
#[must_use]
pub fn d1_modelo(s: &WalState, rec_end: u64, cut: u64) -> bool {
    !(inv_wal(s) && rec_end <= s.acked)
        || !crash_legal(CrashModel::of(s.written, s.synced), cut)
        || cut >= rec_end
}

/// Under `SyncPolicy::Lying` the new record is never acked: the ack may
/// still consume OLD barrier slack (bytes a previous honest sync made
/// durable), but the record's end sits past the unmoved barrier — so D1's
/// premise never fires for it (suspension, not violation).
#[must_use]
pub fn put_lying_never_acks(s0: WalState, rec_len: u64) -> bool {
    let s1 = wal_append(s0, rec_len);
    let s2 = wal_sync(s1, SyncHonesty::Lying);
    let s3 = wal_ack(s2, s2.synced - s2.acked);
    rec_len == 0 || (s3.acked <= s0.synced && s0.synced < s1.written)
}

// --- AS-IS mutants ----------------------------------------------------------

/// AS-IS hole 1: the write path acks without a real barrier (lying sync,
/// unconditional ack of everything appended) — `Ok` is returned for bytes
/// the disk may drop.
#[must_use]
pub fn put_ok_as_is(s0: WalState, rec_len: u64) -> WalState {
    let s1 = wal_append(s0, rec_len);
    let m = env_sync(CrashModel::of(s1.written, s1.synced), SyncHonesty::Lying);
    WalState {
        acked: s1.written,
        synced: m.synced,
        written: s1.written,
    }
}

/// AS-IS hole 2: the corollary under the floor-less legality — torn cuts
/// below the barrier floor are called survivable.
#[must_use]
pub fn d1_modelo_as_is(s: &WalState, rec_end: u64, cut: u64) -> bool {
    !(inv_wal(s) && rec_end <= s.acked)
        || !crash_legal_as_is(CrashModel::of(s.written, s.synced), cut)
        || cut >= rec_end
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wal::wal_state_kernel::wal_state_of;

    #[test]
    fn put_ok_then_every_torn_prefix_keeps_the_record() {
        // A put on an already-durable log: acked covers the record, and
        // the corollary holds for every cut (legal or not).
        let s0 = wal_state_of(64, 64, 64);
        let s = put_ok(s0, 96);
        assert_eq!((s.acked, s.synced, s.written), (160, 160, 160));
        for cut in 0..=s.written + 2 {
            assert!(d1_modelo(&s, 160, cut));
        }
        // And on a cold log.
        let cold = put_ok(wal_state_of(0, 0, 0), 32);
        for cut in 0..=cold.written + 2 {
            assert!(d1_modelo(&cold, 32, cut));
        }
    }

    #[test]
    fn as_is_put_without_barrier_breaks_the_corollary() {
        // AS-IS returns Ok with the barrier unmoved (lying): the crash at
        // cut=0 is legal and drops the "acked" record. The corollary
        // itself refuses the state (Inv-WAL is broken — vacuous truth),
        // which is exactly the contract boundary it should enforce.
        let bad = put_ok_as_is(wal_state_of(0, 0, 0), 96);
        assert_eq!((bad.acked, bad.synced, bad.written), (96, 0, 96));
        assert!(!inv_wal(&bad), "as-is write path violates Inv-WAL");
        assert!(
            crash_legal(CrashModel::of(bad.written, bad.synced), 0) && 0 < 96,
            "legal torn cut at 0 loses the as-is acked put"
        );
        assert!(d1_modelo(&bad, 96, 0), "corollary is vacuous off-contract");
    }

    #[test]
    fn floor_less_legality_diverges_from_the_corollary() {
        // Honest barrier at 160, record ending at 96: cut=64 is a torn
        // prefix below the record — illegal per the real geometry (the
        // corollary is vacuously true), "legal" per the floor-less as-is.
        let s = wal_state_of(160, 160, 160);
        assert!(d1_modelo(&s, 96, 64));
        assert!(!d1_modelo_as_is(&s, 96, 64));
    }

    #[test]
    fn lying_barrier_suspends_the_premise() {
        // Under Lying the new record is never acked — its end sits past
        // the unmoved barrier, so D1's premise never fires for it. An old
        // slack may still be acked (previous honest barrier), never the
        // new bytes.
        let s0 = wal_state_of(10, 10, 4); // slack [4, 10) from an old sync
        assert!(put_lying_never_acks(s0, 96));
        let s1 = wal_append(s0, 96);
        let s2 = wal_sync(s1, SyncHonesty::Lying);
        let s3 = wal_ack(s2, s2.synced - s2.acked);
        assert_eq!(s3.acked, 10, "old slack fully acked, barrier unmoved");
        assert!(s3.acked < 106, "the new record end (106) is never covered");
    }
}
