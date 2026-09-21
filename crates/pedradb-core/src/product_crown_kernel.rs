//! RFC-0215 P1.2: the product crown (twin of the Lean compose lib
//! `ComposeProductCrown`). For every spine-reachable ledger the crown
//! runs BOTH real kernels and checks they agree — the product promise
//! in its two legs:
//!
//! - model leg: [`crate::d1_modelo_kernel::d1_modelo`] over the ledger
//!   state at every torn cut (atom `catalog:d1_modelo`);
//! - spec leg: [`pedradb_spec::properties_kernel::d1_holds`] over the
//!   positional view of the acked prefix (byte `i` is an acked entry
//!   iff `i < acked`; the cut is the surviving prefix) — the predictor
//!   verdict (atom `catalog:d1_durability`, RFC-0215 P0.1).
//!
//! [`product_crown`] asserts both legs agree `true` for every torn
//! cut — fail-closed. The AS-IS twin ([`product_crown_as_is`]) runs
//! the barrier-less ack path ([`crate::write_ack_kernel::write_ack_ledger_as_is`]):
//! the spec leg catches the unsynced acked prefix that
//! `d1_modelo_as_is` still accepts (a cut at the barrier loses it).
//!
//! Single artifact (Aeneas-paid): the rustc body this crate links IS
//! the proof term — the Lean side composes the fate atoms over the
//! Charon+Aeneas extracts (`./scripts/aeneas_write_ack.sh`,
//! `./scripts/aeneas_properties.sh`).

#![forbid(unsafe_code)]

use crate::d1_modelo_kernel::{d1_modelo, d1_modelo_as_is};
use crate::env_crash_kernel::{crash_legal, CrashModel};
#[cfg(pedra_aeneas)]
use crate::properties_kernel::d1_holds;
use crate::wal::wal_state_kernel::WalState;
#[cfg(not(pedra_aeneas))]
use pedradb_spec::properties_kernel::d1_holds;

/// The positional view of the ledger's acked prefix: byte `i` is an
/// acked entry iff `i < acked` (length = the appended log).
#[must_use]
pub fn acked_flags(s: &WalState) -> Vec<bool> {
    // Index `while` (not `map`/`collect`) so Charon/Aeneas emit a `def`.
    let mut flags = Vec::new();
    let mut i = 0u64;
    while i < s.written {
        flags.push(i < s.acked);
        i = i.saturating_add(1);
    }
    flags
}

/// The product crown over a ledger state: for every cut the model
/// verdict holds (fail-closed — vacuous off the legal geometry), and
/// for every LEGAL torn cut (`barrier floor <= cut <= written`) the
/// spec verdict over the positional view holds too — the acked prefix
/// is durable in the machine AND in the predictor.
#[must_use]
pub fn product_crown(s: &WalState) -> bool {
    let flags = acked_flags(s);
    let m = CrashModel::of(s.written, s.synced);
    // Index `while` (not `Iterator::all`) so Charon/Aeneas emit a `def`.
    let last = s.written.saturating_add(2);
    let mut cut = 0u64;
    loop {
        if !d1_modelo(s, s.acked, cut) || (crash_legal(m, cut) && !d1_holds(&flags, cut as usize)) {
            return false;
        }
        if cut == last {
            return true;
        }
        cut = cut.saturating_add(1);
    }
}

/// AS-IS twin: the same crown over the barrier-less ack geometry —
/// `d1_modelo_as_is` accepts the unsynced acked prefix (Inv-WAL broken,
/// vacuous yes), but the spec leg still runs at the real legal cuts and
/// refuses the unsynced acked prefix (a cut at the barrier floor loses
/// the acked bytes).
#[must_use]
pub fn product_crown_as_is(s: &WalState) -> bool {
    let flags = acked_flags(s);
    let m = CrashModel::of(s.written, s.synced);
    let last = s.written.saturating_add(2);
    let mut cut = 0u64;
    loop {
        if !d1_modelo_as_is(s, s.acked, cut)
            || (crash_legal(m, cut) && !d1_holds(&flags, cut as usize))
        {
            return false;
        }
        if cut == last {
            return true;
        }
        cut = cut.saturating_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::durability_spine_kernel::{spine_replay, SpineStep};
    use crate::write_ack_kernel::WriteAckLedger;

    #[test]
    fn crown_agrees_on_every_reachable_ledger() {
        // Interleaved groups with a trailing unsynced append: the crown
        // holds after EVERY spine prefix (any reachable ledger).
        let mut l = WriteAckLedger::new();
        let steps = [
            SpineStep::Append(64),
            SpineStep::Append(32),
            SpineStep::Barrier,
            SpineStep::Ack,
            SpineStep::Append(16),
            SpineStep::Barrier,
            SpineStep::Ack,
            SpineStep::Append(8),
        ];
        for n in 1..=steps.len() {
            let mut li = WriteAckLedger::new();
            spine_replay(&mut li, &steps[..n]);
            let (acked, synced, written) = li.snapshot();
            let s = WalState {
                acked,
                synced,
                written,
            };
            assert!(
                product_crown(&s),
                "crown holds after {n} spine steps ({acked},{synced},{written})"
            );
        }
    }

    #[test]
    fn crown_as_is_breaks_on_unsynced_ack() {
        // The barrier-less ack: acked > synced. The as-is model still
        // says yes; the spec leg catches the loss at the barrier cut.
        let s = WalState {
            acked: 64,
            synced: 0,
            written: 64,
        };
        assert!(
            !product_crown_as_is(&s),
            "spec leg refuses the unsynced ack"
        );
        // The honest geometry on the same bytes passes both legs.
        let honest = WalState {
            acked: 64,
            synced: 64,
            written: 64,
        };
        assert!(product_crown(&honest));
    }
}
