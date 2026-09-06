// Verus twin of the D1-modelo kernel (RFC-0166 P1.3 —
// crates/pedradb-core/src/d1_modelo_kernel.rs). Not linked into production.
//
//   ./scripts/verus_d1_modelo.sh
//
// Theorems: exec == spec for the put write path and the named corollary;
// the corollary itself (put Ok ⇒ survives every legal torn prefix) under
// Inv-WAL; the lying suspension (the new record is never acked when the
// barrier lies); and the AS-IS teeth (ack without a barrier; floor-less
// legality) are witnessed to diverge from the real semantics.

use vstd::prelude::*;

verus! {

#[derive(PartialEq, Eq, Clone, Copy)]
pub enum SyncHonesty {
    Honest,
    Lying,
}

#[derive(PartialEq, Eq, Clone, Copy)]
pub struct WalState {
    pub acked: u64,
    pub synced: u64,
    pub written: u64,
}

// --- restated atoms (twins of env_crash.rs / wal_state.rs) ---------------

pub open spec fn fsync_promotes_spec(os_honest: bool) -> bool {
    os_honest
}

pub open spec fn inv_wal_spec(s: WalState) -> bool {
    s.acked <= s.synced && s.synced <= s.written
}

pub open spec fn crash_legal_spec(s: WalState, cut: u64) -> bool {
    s.synced <= cut && cut <= s.written
}

pub open spec fn wal_append_spec(s: WalState, n: u64) -> WalState {
    WalState { written: (s.written + n) as u64, synced: s.synced, acked: s.acked }
}

pub open spec fn wal_sync_spec(s: WalState, h: SyncHonesty) -> WalState {
    if fsync_promotes_spec(h == SyncHonesty::Honest) {
        WalState { written: s.written, synced: s.written, acked: s.acked }
    } else {
        s
    }
}

pub open spec fn wal_ack_spec(s: WalState, n: u64) -> WalState {
    if (s.acked + n) as u64 <= s.synced {
        WalState { acked: (s.acked + n) as u64, synced: s.synced, written: s.written }
    } else {
        s
    }
}

// --- the put write path and the named corollary ---------------------------

pub open spec fn put_ok_spec(s0: WalState, n: u64) -> WalState {
    // append → honest sync (barrier = whole log) → ack every durable
    // pending byte: the whole log ends inside the acked prefix.
    let s1 = wal_append_spec(s0, n);
    WalState { written: s1.written, synced: s1.written, acked: s1.written }
}

pub open spec fn d1_modelo_spec(s: WalState, rec_end: u64, cut: u64) -> bool {
    !(inv_wal_spec(s) && rec_end <= s.acked)
        || !crash_legal_spec(s, cut)
        || cut >= rec_end
}

/// Model write path of one put: append → honest sync → ack every durable
/// pending byte (group-commit semantics). The caller holds `Ok` in the
/// returned state.
pub fn put_ok(s0: WalState, n: u64) -> (r: WalState)
    requires
        inv_wal_spec(s0),
        n <= u64::MAX - s0.written,
    ensures
        r == put_ok_spec(s0, n),
{
    let s1 = wal_append(s0, n);
    let s2 = wal_sync(s1, SyncHonesty::Honest);
    wal_ack(s2, s2.synced - s2.acked)
}

pub fn wal_append(s: WalState, n: u64) -> (r: WalState)
    requires
        n <= u64::MAX - s.written,
    ensures
        r == wal_append_spec(s, n),
{
    WalState { written: s.written + n, synced: s.synced, acked: s.acked }
}

pub fn wal_sync(s: WalState, h: SyncHonesty) -> (r: WalState)
    ensures
        r == wal_sync_spec(s, h),
{
    if fsync_promotes_spec_is_honest(h) {
        WalState { written: s.written, synced: s.written, acked: s.acked }
    } else {
        s
    }
}

fn fsync_promotes_spec_is_honest(h: SyncHonesty) -> (b: bool)
    ensures
        b == fsync_promotes_spec(h == SyncHonesty::Honest),
{
    match h {
        SyncHonesty::Honest => true,
        SyncHonesty::Lying => false,
    }
}

pub fn wal_ack(s: WalState, n: u64) -> (r: WalState)
    requires
        n <= u64::MAX - s.acked,
    ensures
        r == wal_ack_spec(s, n),
{
    if s.acked + n <= s.synced {
        WalState { acked: s.acked + n, synced: s.synced, written: s.written }
    } else {
        s
    }
}

/// Named corollary D1-modelo: a record inside the acked prefix of an
/// Inv-WAL state survives every legal torn prefix.
pub fn d1_modelo(s: WalState, rec_end: u64, cut: u64) -> (b: bool)
    ensures
        b == d1_modelo_spec(s, rec_end, cut),
{
    !(s.acked <= s.synced && s.synced <= s.written && rec_end <= s.acked)
        || !(s.synced <= cut && cut <= s.written)
        || cut >= rec_end
}

/// Under `SyncPolicy::Lying` the new record is never acked (rec_len > 0):
/// its end sits past the unmoved barrier.
pub fn put_lying_never_acks(s0: WalState, rec_len: u64) -> (b: bool)
    requires
        inv_wal_spec(s0),
        rec_len <= u64::MAX - s0.written,
    ensures
        b == (rec_len == 0
            || {
                let s1 = wal_append_spec(s0, rec_len);
                let s2 = wal_sync_spec(s1, SyncHonesty::Lying);
                let s3 = wal_ack_spec(s2, (s2.synced - s2.acked) as u64);
                s3.acked <= s0.synced && s0.synced < s1.written
            }),
{
    let s1 = wal_append(s0, rec_len);
    let s2 = wal_sync(s1, SyncHonesty::Lying);
    let s3 = wal_ack(s2, s2.synced - s2.acked);
    rec_len == 0 || (s3.acked <= s0.synced && s0.synced < s1.written)
}

// --- The named corollary ----------------------------------------------------

/// D1-modelo (the named theorem of RFC-0166 P1.3): put Ok ⇒ survives every
/// torn prefix. Under Inv-WAL, a record ending at or before `acked` is
/// kept whole by every legal crash cut: the cut's floor is `synced`, and
/// `rec_end <= acked <= synced <= cut`.
proof fn d1_modelo_theorem(s: WalState, rec_end: u64, cut: u64)
    requires
        inv_wal_spec(s),
        rec_end <= s.acked,
        crash_legal_spec(s, cut),
    ensures
        cut >= rec_end,
{
}

/// The write path establishes the premise: after `put_ok` the whole log —
/// the new record ([s0.written, s0.written + n)) included — lies inside
/// the acked prefix of a state still satisfying Inv-WAL.
proof fn put_ok_establishes_premise(s0: WalState, n: u64)
    requires
        inv_wal_spec(s0),
        n <= u64::MAX - s0.written,
    ensures
        inv_wal_spec(put_ok_spec(s0, n)),
        (s0.written + n) as u64 <= put_ok_spec(s0, n).acked,
{
}

/// Lying suspension: with a lying barrier and a non-empty record, the
/// record end is never covered by the acked prefix — D1's premise never
/// fires for the new put.
proof fn lying_suspends_the_premise(s0: WalState, n: u64)
    requires
        inv_wal_spec(s0),
        n > 0,
        n <= u64::MAX - s0.written,
    ensures
        ({
            let s1 = wal_append_spec(s0, n);
            let s2 = wal_sync_spec(s1, SyncHonesty::Lying);
            let s3 = wal_ack_spec(s2, (s2.synced - s2.acked) as u64);
            s3.acked < (s0.written + n) as u64
        }),
{
}

// --- AS-IS teeth ------------------------------------------------------------

/// AS-IS 1: the write path acks without a real barrier (lying sync,
/// unconditional ack of everything appended) — `Ok` is returned for bytes
/// the disk may drop.
pub open spec fn put_ok_as_is_spec(s0: WalState, n: u64) -> WalState {
    WalState {
        acked: wal_append_spec(s0, n).written,
        synced: wal_append_spec(s0, n).synced,
        written: wal_append_spec(s0, n).written,
    }
}

proof fn put_as_is_breaks_inv_wal()
    ensures
        inv_wal_spec(WalState { acked: 0, synced: 0, written: 0 }),
        !inv_wal_spec(put_ok_as_is_spec(WalState { acked: 0, synced: 0, written: 0 }, 96)),
        crash_legal_spec(put_ok_as_is_spec(WalState { acked: 0, synced: 0, written: 0 }, 96), 0),
        put_ok_as_is_spec(WalState { acked: 0, synced: 0, written: 0 }, 96).acked > 0,
{
}

/// AS-IS 2: the corollary under the floor-less legality — torn cuts below
/// the barrier floor are called survivable.
pub open spec fn d1_modelo_as_is_spec(s: WalState, rec_end: u64, cut: u64) -> bool {
    !(inv_wal_spec(s) && rec_end <= s.acked)
        || !(cut <= s.written)
        || cut >= rec_end
}

proof fn floor_less_legality_diverges()
    ensures
        inv_wal_spec(WalState { acked: 160, synced: 160, written: 160 }),
        d1_modelo_spec(WalState { acked: 160, synced: 160, written: 160 }, 96, 64),
        !d1_modelo_as_is_spec(WalState { acked: 160, synced: 160, written: 160 }, 96, 64),
{
}

} // verus!
