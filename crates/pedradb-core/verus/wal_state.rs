// Verus twin of the WAL-state kernel (RFC-0166 P1.2 —
// crates/pedradb-core/src/wal/wal_state_kernel.rs). Not linked into
// production.
//
//   ./scripts/verus_wal_state.sh
//
// Theorems: exec == spec for Inv-WAL and its four atoms; Inv-WAL is
// preserved by append/sync/ack/rotate; the survival bridge (every legal
// crash cut keeps the acked prefix); and the AS-IS teeth (ack before the
// barrier, ack past the barrier, rotate with a non-durable tail, and the
// floor-less survival legality) are witnessed to diverge from the real
// semantics.

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

// The proved group_commit kernel fact, restated for this crate-local twin.
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

pub open spec fn wal_rotate_spec(s: WalState) -> WalState {
    if s.acked == s.synced && s.synced == s.written {
        WalState { acked: 0, synced: 0, written: 0 }
    } else {
        s
    }
}

pub open spec fn acked_survives_spec(s: WalState, cut: u64) -> bool {
    !crash_legal_spec(s, cut) || cut >= s.acked
}

pub fn inv_wal(s: WalState) -> (b: bool)
    ensures
        b == inv_wal_spec(s),
{
    s.acked <= s.synced && s.synced <= s.written
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

pub fn wal_rotate(s: WalState) -> (r: WalState)
    ensures
        r == wal_rotate_spec(s),
{
    if s.acked == s.synced && s.synced == s.written {
        WalState { acked: 0, synced: 0, written: 0 }
    } else {
        s
    }
}

pub fn acked_survives_every_legal_crash(s: WalState, cut: u64) -> (b: bool)
    ensures
        b == acked_survives_spec(s, cut),
{
    !(s.synced <= cut && cut <= s.written) || cut >= s.acked
}

// --- Preservation theorems --------------------------------------------------

/// Inv-WAL is preserved by append (neither barrier nor acked prefix move).
proof fn append_preserves_inv(s: WalState, n: u64)
    requires
        inv_wal_spec(s),
        n <= u64::MAX - s.written,
    ensures
        inv_wal_spec(wal_append_spec(s, n)),
{
}

/// Inv-WAL is preserved by sync for both honesties: honest sync only raises
/// the barrier to `written` (acked still below it); lying sync changes
/// nothing.
proof fn sync_preserves_inv(s: WalState, h: SyncHonesty)
    requires
        inv_wal_spec(s),
    ensures
        inv_wal_spec(wal_sync_spec(s, h)),
{
}

/// Inv-WAL is preserved by ack: a legal ack stays under the barrier, an
/// illegal one is refused (state unchanged).
proof fn ack_preserves_inv(s: WalState, n: u64)
    requires
        inv_wal_spec(s),
        n <= u64::MAX - s.acked,
    ensures
        inv_wal_spec(wal_ack_spec(s, n)),
{
}

/// Inv-WAL is preserved by rotate: the only dropped log is the fully
/// durable-and-acked one; the empty state satisfies Inv-WAL.
proof fn rotate_preserves_inv(s: WalState)
    requires
        inv_wal_spec(s),
    ensures
        inv_wal_spec(wal_rotate_spec(s)),
{
}

/// Survival bridge: under Inv-WAL every legal crash cut keeps at least the
/// acked prefix (the floor of a legal cut is `synced >= acked`).
proof fn inv_wal_survives_theorem(s: WalState, cut: u64)
    requires
        inv_wal_spec(s),
    ensures
        crash_legal_spec(s, cut) ==> cut >= s.acked,
{
}

// --- AS-IS teeth ------------------------------------------------------------

/// AS-IS 1: append acks the bytes together with the write — before any
/// barrier.
pub open spec fn wal_append_as_is_spec(s: WalState, n: u64) -> WalState {
    WalState { acked: (s.acked + n) as u64, synced: s.synced, written: (s.written + n) as u64 }
}

proof fn append_as_is_breaks_inv()
    ensures
        inv_wal_spec(WalState { acked: 0, synced: 0, written: 0 }),
        !inv_wal_spec(wal_append_as_is_spec(WalState { acked: 0, synced: 0, written: 0 }, 8)),
{
}

/// AS-IS 2: ack advances past the barrier unconditionally.
pub open spec fn wal_ack_as_is_spec(s: WalState, n: u64) -> WalState {
    WalState { acked: (s.acked + n) as u64, synced: s.synced, written: s.written }
}

proof fn ack_as_is_breaks_inv_and_survival()
    ensures
        inv_wal_spec(WalState { acked: 2, synced: 3, written: 10 }),
        wal_ack_spec(WalState { acked: 2, synced: 3, written: 10 }, 5)
            == (WalState { acked: 2, synced: 3, written: 10 }),
        !inv_wal_spec(wal_ack_as_is_spec(WalState { acked: 2, synced: 3, written: 10 }, 5)),
        crash_legal_spec(wal_ack_as_is_spec(WalState { acked: 2, synced: 3, written: 10 }, 5), 3),
{
}

/// AS-IS 3: rotate drops the log even with a non-durable tail.
pub open spec fn wal_rotate_as_is_spec(s: WalState) -> WalState {
    WalState { acked: 0, synced: 0, written: 0 }
}

proof fn rotate_as_is_loses_acked_bytes()
    ensures
        inv_wal_spec(WalState { acked: 3, synced: 3, written: 10 }),
        wal_rotate_spec(WalState { acked: 3, synced: 3, written: 10 })
            == (WalState { acked: 3, synced: 3, written: 10 }),
        wal_rotate_as_is_spec(WalState { acked: 3, synced: 3, written: 10 }).acked < 3,
{
}

/// AS-IS 4: floor-less survival legality — accepts cuts below the barrier
/// as "survivable" and then loses the acked prefix.
pub open spec fn acked_survives_as_is_spec(s: WalState, cut: u64) -> bool {
    !(cut <= s.written) || cut >= s.acked
}

proof fn survival_as_is_diverges()
    ensures
        acked_survives_spec(WalState { acked: 3, synced: 3, written: 10 }, 1),
        !acked_survives_as_is_spec(WalState { acked: 3, synced: 3, written: 10 }, 1),
{
}

} // verus!
