//! The verified write→ack ledger (RFC-0166 P1.4): the pinned profile's
//! durable-group critical section advanced **by the proved kernels**.
//!
//! The live critical section (`concurrent.rs`: `write_pending_frame` →
//! `sync_data` → `may_publish_group`/`group_apply`) reports each step here
//! when the verified pin is set, and the ledger moves exactly as
//! [`crate::wal::wal_state_kernel`] and [`crate::d1_modelo_kernel`]
//! prescribe — no raw arithmetic, only proved atoms:
//!
//! - [`WriteAckLedger::on_append`]: bytes entered the WAL (written grows);
//! - [`WriteAckLedger::on_barrier`]: fdatasync Ok — honest seam
//!   ([`crate::env_crash_kernel::SyncHonesty::Honest`]); the ledger models
//!   the barrier as honest (D1 is conditional on barrier honesty);
//! - [`WriteAckLedger::on_ack`]: the group published — every durable
//!   pending byte acked (group-commit semantics, `put_ok` shape);
//! - [`WriteAckLedger::assert_inv`]: fail-closed Inv-WAL check;
//! - [`WriteAckLedger::d1_holds_every_cut`]: the D1-modelo corollary over
//!   every torn prefix up to `through`.
//!
//! AS-IS tooth: acking the group before the barrier
//! ([`write_ack_ledger_as_is`]) breaks Inv-WAL on the spot. Single
//! artifact (Aeneas-paid): the rustc body this crate links IS the proof
//! term — theorems over the Charon+Aeneas extract
//! (`./scripts/aeneas_write_ack.sh`).

#![forbid(unsafe_code)]

use crate::d1_modelo_kernel::d1_modelo;
use crate::env_crash_kernel::SyncHonesty;
use crate::wal::wal_state_kernel::{inv_wal, wal_ack, wal_append, wal_sync, WalState};

/// Ledger of the verified write→ack path. Pure model state: the caller
/// (the pinned commit section) reports each step; nothing here touches
/// I/O.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WriteAckLedger {
    state: WalState,
}

impl WriteAckLedger {
    /// Cold ledger: empty log, no barrier, no ack.
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: WalState {
                acked: 0,
                synced: 0,
                written: 0,
            },
        }
    }

    /// Bytes entered the WAL (the frame write happened; barrier not yet).
    pub fn on_append(&mut self, bytes: u64) {
        self.state = wal_append(self.state, bytes);
    }

    /// The fdatasync barrier returned Ok (honest seam — see the module
    /// docs for the Lying boundary).
    pub fn on_barrier(&mut self) {
        self.state = wal_sync(self.state, SyncHonesty::Honest);
    }

    /// The group was published: every durable pending byte is acked
    /// (the `put_ok` composition).
    pub fn on_ack(&mut self) {
        let pending = self.state.synced - self.state.acked;
        self.state = wal_ack(self.state, pending);
    }

    /// Fail-closed Inv-WAL check for the live path.
    ///
    /// # Panics
    /// If the live path ever leaves the proved geometry.
    pub fn assert_inv(&self) {
        assert!(inv_wal(&self.state), "verified write→ack left Inv-WAL");
    }

    /// D1-modelo over every torn prefix up to and past `through` (the
    /// record = the whole acked prefix).
    #[must_use]
    pub fn d1_holds_every_cut(&self, through: u64) -> bool {
        (0..=(through.saturating_add(2))).all(|cut| d1_modelo(&self.state, self.state.acked, cut))
    }

    /// Snapshot `(acked, synced, written)`.
    #[must_use]
    pub fn snapshot(&self) -> (u64, u64, u64) {
        (self.state.acked, self.state.synced, self.state.written)
    }
}

impl Default for WriteAckLedger {
    fn default() -> Self {
        Self::new()
    }
}

/// AS-IS tooth: the ledger acks the appended bytes with no barrier —
/// Inv-WAL breaks immediately (an unsynced acked prefix).
#[must_use]
pub fn write_ack_ledger_as_is(mut l: WriteAckLedger, bytes: u64) -> WriteAckLedger {
    l.on_append(bytes);
    l.state = crate::wal::wal_state_kernel::wal_ack_as_is(l.state, bytes);
    l
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wal::wal_state_kernel::wal_state_of;

    #[test]
    fn ledger_group_is_the_put_ok_composition() {
        // One durable group on a cold log: append → barrier → ack lands
        // exactly on the `put_ok` geometry (acked == synced == written).
        let mut l = WriteAckLedger::new();
        l.on_append(96);
        l.assert_inv();
        assert_eq!(l.snapshot(), (0, 0, 96));
        l.on_barrier();
        l.assert_inv();
        assert_eq!(l.snapshot(), (0, 96, 96));
        l.on_ack();
        l.assert_inv();
        assert_eq!(l.snapshot(), (96, 96, 96));
        assert!(l.d1_holds_every_cut(96));
        // Two more groups (partial barriers): Inv-WAL holds throughout.
        l.on_append(64);
        l.on_barrier();
        l.on_ack();
        l.on_append(32);
        l.on_barrier();
        l.on_ack();
        l.assert_inv();
        assert_eq!(l.snapshot(), (192, 192, 192));
        assert!(l.d1_holds_every_cut(192));
    }

    #[test]
    fn append_without_barrier_keeps_inv_and_suspends_d1() {
        // Appended-but-unbarriered bytes keep Inv-WAL; the D1 corollary
        // still holds for the acked prefix over every cut (the unacked
        // tail is not claimed).
        let mut l = WriteAckLedger::new();
        l.on_append(64);
        l.on_barrier();
        l.on_ack();
        l.on_append(32); // pending, no barrier yet
        l.assert_inv();
        assert_eq!(l.snapshot(), (64, 64, 96));
        assert!(l.d1_holds_every_cut(96));
    }

    #[test]
    fn as_is_ack_before_barrier_breaks_inv() {
        let bad = write_ack_ledger_as_is(WriteAckLedger::new(), 96);
        assert_eq!(bad.snapshot(), (96, 0, 96), "as-is acked without a barrier");
        let (acked, synced, written) = bad.snapshot();
        assert!(!(acked <= synced && synced <= written));
    }

    #[test]
    fn ledger_equals_wal_state_chain() {
        // The ledger is exactly the wal_state kernel chain — no extra
        // state, no extra arithmetic.
        let mut l = WriteAckLedger::new();
        l.on_append(10);
        l.on_barrier();
        l.on_ack();
        let chain = wal_ack(
            wal_sync(wal_append(wal_state_of(0, 0, 0), 10), SyncHonesty::Honest),
            10,
        );
        assert_eq!(l.snapshot(), (chain.acked, chain.synced, chain.written));
    }

    /// RFC-0166 P1.4 G1 sanity: the ledger is pin-gated by construction —
    /// this bounds what a pinned durable group can add. A group is one
    /// mutex lock + three kernel steps + the assert; anything near a
    /// fdatasync (~tens of µs) would be a real regression signal.
    #[test]
    fn ledger_step_cost_bound() {
        let mut l = WriteAckLedger::new();
        // Warm.
        for _ in 0..1000 {
            l.on_append(64);
            l.on_barrier();
            l.on_ack();
        }
        let t0 = std::time::Instant::now();
        const GROUPS: u64 = 100_000;
        for _ in 0..GROUPS {
            l.on_append(64);
            l.on_barrier();
            l.on_ack();
        }
        let per_group_ns = t0.elapsed().as_nanos() as u64 / GROUPS;
        println!("ledger per-durable-group cost: {per_group_ns} ns (no lock)");
        assert!(
            per_group_ns < 10_000,
            "ledger step cost {per_group_ns} ns/group is not pin-cheap"
        );
    }
}
