//! RFC-0214 P2.1: the composed durability spine (twin of the Lean
//! compose lib `ComposeDurabilitySpine`). Each spine step is the Ok
//! future of its registered atom (`catalog:write_ack_append`,
//! `catalog:write_ack_barrier`, `catalog:write_ack_ack`); the
//! composition folds the REAL ledger kernel
//! ([`crate::write_ack_kernel::WriteAckLedger`]) over ANY step
//! sequence and checks the composed sentence fail-closed:
//!
//! - Inv-WAL (`acked ⊆ synced ⊆ written`) holds after EVERY step —
//!   [`spine_replay`] asserts per step;
//! - D1: every acked prefix survives every torn cut — the caller
//!   folds [`WriteAckLedger::d1_holds_every_cut`] over the replay;
//! - AS-IS twin: the barrier never runs — [`spine_replay_as_is`]
//!   acks on append and breaks Inv-WAL on the first group.
//!
//! Single artifact (Aeneas-paid): the rustc body this crate links IS
//! the proof term — the Lean side composes the fate atoms over the
//! Charon+Aeneas extract (`./scripts/aeneas_write_ack.sh`).

#![forbid(unsafe_code)]

use crate::write_ack_kernel::{write_ack_ledger_as_is, WriteAckLedger};

/// One step of the durability spine (the three promoted atoms).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpineStep {
    /// Bytes entered the WAL (atom `write_ack_append`).
    Append(u64),
    /// The honest fdatasync barrier (atom `write_ack_barrier`).
    Barrier,
    /// The group published — the durable gap acks (atom
    /// `write_ack_ack`).
    Ack,
}

/// Replay ANY spine step sequence through the real ledger kernel,
/// asserting Inv-WAL after every step — the composed invariant,
/// fail-closed.
///
/// # Panics
/// If any prefix of the sequence leaves the proved geometry.
pub fn spine_replay(l: &mut WriteAckLedger, steps: &[SpineStep]) {
    for s in steps {
        match s {
            SpineStep::Append(n) => l.on_append(*n),
            SpineStep::Barrier => l.on_barrier(),
            SpineStep::Ack => l.on_ack(),
        }
        l.assert_inv();
    }
}

/// AS-IS twin of the composed path: the barrier never runs — each
/// append acks itself (the lying `write_ack_ledger_as_is` seam).
/// Returns the final snapshot; the caller asserts Inv-WAL broke.
#[must_use]
pub fn spine_replay_as_is(l: &mut WriteAckLedger, steps: &[SpineStep]) -> (u64, u64, u64) {
    for s in steps {
        match s {
            SpineStep::Append(n) => *l = write_ack_ledger_as_is(l.clone(), *n),
            SpineStep::Barrier | SpineStep::Ack => {}
        }
    }
    l.snapshot()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spine_any_sequence_keeps_inv_and_d1() {
        // Interleaved groups with a trailing unsynced append: Inv-WAL
        // holds after every step and D1 holds over every torn cut.
        let mut l = WriteAckLedger::new();
        spine_replay(
            &mut l,
            &[
                SpineStep::Append(64),
                SpineStep::Append(32),
                SpineStep::Barrier,
                SpineStep::Ack,
                SpineStep::Append(16),
                SpineStep::Barrier,
                SpineStep::Ack,
                SpineStep::Append(8),
            ],
        );
        assert_eq!(
            l.snapshot(),
            (112, 112, 120),
            "groups ack, tail stays pending"
        );
        assert!(l.d1_holds_every_cut(120), "D1 over every torn prefix");
    }

    #[test]
    fn spine_as_is_breaks_inv_on_first_group() {
        let mut l = WriteAckLedger::new();
        let snap = spine_replay_as_is(
            &mut l,
            &[SpineStep::Append(64), SpineStep::Barrier, SpineStep::Ack],
        );
        assert_eq!(snap, (64, 0, 64), "as-is acks with no barrier");
        assert!(snap.0 > snap.1, "Inv-WAL broken: unsynced acked prefix");
    }
}
