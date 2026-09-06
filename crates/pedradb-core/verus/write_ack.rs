// Verus twin of the write→ack ledger kernel (RFC-0166 P1.4 —
// crates/pedradb-core/src/write_ack_kernel.rs). Not linked into production.
//
//   ./scripts/verus_write_ack.sh
//
// Theorems: the ledger group step (append → barrier → ack) lands exactly
// on the put_ok composition (acked == synced == written); Inv-WAL holds
// after every ledger step; the D1-modelo corollary transfers to the
// ledger over every cut; and the AS-IS tooth (ack before the barrier)
// breaks Inv-WAL.

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

// --- restated atoms ---------------------------------------------------------

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

pub open spec fn d1_modelo_spec(s: WalState, rec_end: u64, cut: u64) -> bool {
    !(inv_wal_spec(s) && rec_end <= s.acked)
        || !crash_legal_spec(s, cut)
        || cut >= rec_end
}

// --- the ledger -------------------------------------------------------------

pub struct WriteAckLedger {
    pub state: WalState,
}

pub open spec fn ledger_new_spec() -> WriteAckLedger {
    WriteAckLedger { state: WalState { acked: 0, synced: 0, written: 0 } }
}

pub open spec fn on_append_spec(l: WriteAckLedger, bytes: u64) -> WriteAckLedger {
    WriteAckLedger { state: wal_append_spec(l.state, bytes) }
}

pub open spec fn on_barrier_spec(l: WriteAckLedger) -> WriteAckLedger {
    WriteAckLedger { state: wal_sync_spec(l.state, SyncHonesty::Honest) }
}

pub open spec fn on_ack_spec(l: WriteAckLedger) -> WriteAckLedger {
    let pending = (l.state.synced - l.state.acked) as u64;
    WriteAckLedger { state: wal_ack_spec(l.state, pending) }
}

/// One durable group on the ledger: append → barrier → ack.
pub open spec fn ledger_group_spec(l: WriteAckLedger, bytes: u64) -> WriteAckLedger {
    on_ack_spec(on_barrier_spec(on_append_spec(l, bytes)))
}

/// The `put_ok` composition over a raw WalState (twin of d1_modelo.rs).
pub open spec fn put_ok_spec(s0: WalState, n: u64) -> WalState {
    WalState {
        written: (s0.written + n) as u64,
        synced: (s0.written + n) as u64,
        acked: (s0.written + n) as u64,
    }
}

pub fn new_ledger() -> (l: WriteAckLedger)
    ensures
        l == ledger_new_spec(),
{
    WriteAckLedger { state: WalState { acked: 0, synced: 0, written: 0 } }
}

pub fn on_append(l: WriteAckLedger, bytes: u64) -> (out: WriteAckLedger)
    requires
        bytes <= u64::MAX - l.state.written,
    ensures
        out == on_append_spec(l, bytes),
{
    WriteAckLedger {
        state: WalState {
            written: l.state.written + bytes,
            synced: l.state.synced,
            acked: l.state.acked,
        },
    }
}

pub fn on_barrier(l: WriteAckLedger) -> (out: WriteAckLedger)
    ensures
        out == on_barrier_spec(l),
{
    WriteAckLedger {
        state: WalState {
            written: l.state.written,
            synced: l.state.written,
            acked: l.state.acked,
        },
    }
}

pub fn on_ack(l: WriteAckLedger) -> (out: WriteAckLedger)
    requires
        inv_wal_spec(l.state),
    ensures
        out == on_ack_spec(l),
{
    let pending = l.state.synced - l.state.acked;
    let acked = l.state.acked + pending;
    WriteAckLedger {
        state: WalState {
            acked,
            synced: l.state.synced,
            written: l.state.written,
        },
    }
}

/// Fail-closed Inv-WAL check (the live path's geometry gate).
pub fn assert_inv_holds(l: WriteAckLedger) -> (b: bool)
    ensures
        b == inv_wal_spec(l.state),
{
    l.state.acked <= l.state.synced && l.state.synced <= l.state.written
}

/// D1-modelo over one torn prefix cut (the every-cut loop is the twin of
/// the kernel's range loop — one cut is the universal step).
pub fn d1_holds_cut(l: WriteAckLedger, cut: u64) -> (b: bool)
    ensures
        b == d1_modelo_spec(l.state, l.state.acked, cut),
{
    !(l.state.acked <= l.state.synced && l.state.synced <= l.state.written
        && l.state.acked <= l.state.acked)
        || !(l.state.synced <= cut && cut <= l.state.written)
        || cut >= l.state.acked
}

// --- Theorems ---------------------------------------------------------------

/// A ledger group is exactly the `put_ok` composition: after append →
/// barrier → ack the whole log is inside the acked prefix.
proof fn ledger_group_is_put_ok(bytes: u64)
    ensures
        ledger_group_spec(ledger_new_spec(), bytes).state
            == put_ok_spec(WalState { acked: 0, synced: 0, written: 0 }, bytes),
{
}

/// Inv-WAL is preserved by every ledger step of a durable group, and the
/// final acked prefix is the whole log (the D1 premise for the group).
proof fn ledger_group_preserves_inv(l: WriteAckLedger, bytes: u64)
    requires
        inv_wal_spec(l.state),
        bytes <= u64::MAX - l.state.written,
    ensures
        inv_wal_spec(on_append_spec(l, bytes).state),
        inv_wal_spec(on_barrier_spec(on_append_spec(l, bytes)).state),
        inv_wal_spec(ledger_group_spec(l, bytes).state),
        ledger_group_spec(l, bytes).state.acked == ledger_group_spec(l, bytes).state.written,
{
}

/// D1 transfer: for a well-formed ledger and any cut, the corollary holds
/// for the ledger's own acked prefix.
proof fn ledger_d1_transfers(l: WriteAckLedger, cut: u64)
    requires
        inv_wal_spec(l.state),
    ensures
        d1_modelo_spec(l.state, l.state.acked, cut),
{
}

// --- AS-IS tooth ------------------------------------------------------------

/// AS-IS: the ledger acks the appended bytes with no barrier.
pub open spec fn wal_ack_as_is_spec(s: WalState, n: u64) -> WalState {
    WalState { acked: (s.acked + n) as u64, synced: s.synced, written: s.written }
}

pub open spec fn ledger_as_is_spec(l: WriteAckLedger, bytes: u64) -> WriteAckLedger {
    WriteAckLedger {
        state: wal_ack_as_is_spec(on_append_spec(l, bytes).state, bytes),
    }
}

proof fn as_is_ack_before_barrier_breaks_inv(bytes: u64)
    requires
        bytes > 0,
    ensures
        inv_wal_spec(ledger_new_spec().state),
        !inv_wal_spec(ledger_as_is_spec(ledger_new_spec(), bytes).state),
{
}

} // verus!
