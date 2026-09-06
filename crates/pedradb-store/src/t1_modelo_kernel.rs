//! Named T1 corollary (RFC-0166 P2.2): **TX all-or-nothing as a
//! refinement of the existing fence / abort / recover atoms**.
//!
//! Composes the production TX kernels already on the catalog:
//!
//! - [`crate::txn_kernel::txn_commit_action`]: a fenced abort reverts, it
//!   never materialises user keys.
//! - [`crate::txn_kernel::revert_clears_status`]: an abort **keeps** the
//!   fence so a later `TxnCommit` replay still sees abort.
//! - [`crate::txn_kernel::leftover_txn_is_aborted`]: a leftover prepared
//!   (or mid-apply) TX after crash is aborted — no coordinator log.
//!
//! Inv-TX: never both committed and aborted; committed ⇒ every staged
//! write visible; otherwise nothing visible; aborted ⇒ the fence is on.
//! Mid-apply (partial visible, not yet committed) is **off** Inv-TX —
//! recover must restore it.
//!
//! - [`tx_abort`]: revert + keep the fence.
//! - [`tx_recover`]: leftover / mid-apply becomes aborted (visible = 0,
//!   fenced). The AS-IS leftover leaves the partial visibility in place.
//! - [`t1_modelo`]: after recover, T1 holds — even if the pre-state was
//!   a mid-apply crash.
//!
//! Verus twin: `crates/pedradb-store/verus/t1_modelo.rs`
//! (`scripts/verus_t1_modelo.sh`).

#![forbid(unsafe_code)]

use crate::txn_kernel::{
    leftover_txn_is_aborted, leftover_txn_is_aborted_as_is, revert_clears_status,
    txn_commit_action, TxnCommitAction,
};

/// Abstract TX state: staged writes vs currently visible writes, plus
/// the commit/abort/fence bits the recover path consults.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TxState {
    /// Number of writes the TX staged (prepare).
    pub staged: u64,
    /// Number of staged writes currently visible (0 or `staged` under Inv-TX).
    pub visible: u64,
    /// The TX committed (all staged writes visible).
    pub committed: bool,
    /// The TX aborted (nothing visible, fence on).
    pub aborted: bool,
    /// Abort fence on disk (kept so a later commit replay still reverts).
    pub fenced: bool,
}

/// Empty TX (no staged writes, not committed, not aborted).
#[must_use]
pub fn tx_state_of() -> TxState {
    TxState {
        staged: 0,
        visible: 0,
        committed: false,
        aborted: false,
        fenced: false,
    }
}

/// Inv-TX: all-or-nothing plus the abort fence.
#[must_use]
pub fn inv_tx(s: &TxState) -> bool {
    !(s.committed && s.aborted)
        && s.visible <= s.staged
        && (!s.committed || (s.visible == s.staged && !s.aborted))
        && (!s.aborted || (s.visible == 0 && s.fenced))
        && (s.committed || s.aborted || s.visible == 0)
}

/// T1 on a state: never both committed and aborted; committed ⇒ all
/// staged writes visible; otherwise nothing visible. (The fence is
/// Inv-TX, not T1 — T1 is the observability claim.)
#[must_use]
pub fn t1_holds_of(s: &TxState) -> bool {
    !(s.committed && s.aborted)
        && s.visible <= s.staged
        && (!s.committed || s.visible == s.staged)
        && (s.committed || s.visible == 0)
}

/// Stage one more write. Refused on a finished TX.
#[must_use]
pub fn tx_stage(s: TxState) -> TxState {
    if s.committed || s.aborted {
        s
    } else {
        TxState {
            staged: s.staged.saturating_add(1),
            ..s
        }
    }
}

/// Mid-commit: expose one more staged write. Off Inv-TX until
/// [`tx_commit`] or [`tx_recover`].
#[must_use]
pub fn tx_apply_one(s: TxState) -> TxState {
    if s.committed || s.aborted || s.visible >= s.staged {
        s
    } else {
        TxState {
            visible: s.visible.saturating_add(1),
            ..s
        }
    }
}

/// Finish commit only when every staged write is already visible.
#[must_use]
pub fn tx_commit(s: TxState) -> TxState {
    if s.aborted || s.committed || s.visible != s.staged {
        s
    } else {
        TxState {
            committed: true,
            aborted: false,
            fenced: false,
            ..s
        }
    }
}

/// Abort: revert (nothing visible) and **keep the fence** so a later
/// `TxnCommit` replay still sees abort. Ties
/// [`txn_commit_action`] (Revert) and [`revert_clears_status`] (fence stays).
#[must_use]
pub fn tx_abort(s: TxState) -> TxState {
    if s.committed {
        s
    } else {
        debug_assert_eq!(txn_commit_action(true), TxnCommitAction::Revert);
        debug_assert!(!revert_clears_status(true, true));
        TxState {
            staged: s.staged,
            visible: 0,
            committed: false,
            aborted: true,
            fenced: true,
        }
    }
}

/// AS-IS abort: revert the user keys but drop the fence — a later
/// commit replay materialises the aborted TX.
#[must_use]
pub fn tx_abort_as_is(s: TxState) -> TxState {
    if s.committed {
        s
    } else {
        TxState {
            staged: s.staged,
            visible: 0,
            committed: false,
            aborted: true,
            fenced: false,
        }
    }
}

/// Recover: leftover prepared / mid-apply TX is aborted
/// ([`leftover_txn_is_aborted`]). A committed TX is left alone.
#[must_use]
pub fn tx_recover(s: TxState) -> TxState {
    if s.committed {
        s
    } else if leftover_txn_is_aborted() {
        TxState {
            staged: s.staged,
            visible: 0,
            committed: false,
            aborted: true,
            fenced: true,
        }
    } else {
        s
    }
}

/// AS-IS recover: leftover intents stay live ([`leftover_txn_is_aborted_as_is`]
/// is false) — a mid-apply crash keeps its partial visibility.
#[must_use]
pub fn tx_recover_as_is(s: TxState) -> TxState {
    if leftover_txn_is_aborted_as_is() {
        tx_recover(s)
    } else {
        s
    }
}

/// Named corollary T1-modelo: after recover, T1 holds.
#[must_use]
pub fn t1_modelo(s: TxState) -> bool {
    t1_holds_of(&tx_recover(s))
}

/// AS-IS corollary: T1 claimed of the unrecovered state (mid-apply
/// partial visibility passes).
#[must_use]
pub fn t1_modelo_as_is(s: TxState) -> bool {
    t1_holds_of(&tx_recover_as_is(s))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mid_apply() -> TxState {
        let s0 = tx_state_of();
        let s1 = tx_stage(s0);
        let s2 = tx_stage(s1);
        tx_apply_one(s2) // staged=2, visible=1, not committed
    }

    #[test]
    fn honest_recover_of_mid_apply_restores_t1() {
        let s = mid_apply();
        assert!(!inv_tx(&s), "mid-apply is off Inv-TX: {s:?}");
        assert!(!t1_holds_of(&s), "partial visibility is not T1");
        let r = tx_recover(s);
        assert!(inv_tx(&r), "recover restores Inv-TX: {r:?}");
        assert!(t1_holds_of(&r));
        assert!(r.aborted && r.fenced && r.visible == 0);
        assert!(t1_modelo(s));
    }

    #[test]
    fn as_is_recover_of_mid_apply_leaves_partial() {
        let s = mid_apply();
        let bad = tx_recover_as_is(s);
        assert_eq!(bad, s, "AS-IS leftover leaves the mid-apply state");
        assert!(!t1_holds_of(&bad));
        assert!(!t1_modelo_as_is(s), "AS-IS corollary claims T1 of partial");
        assert!(t1_modelo(s));
    }

    #[test]
    fn abort_keeps_the_fence_and_hides_everything() {
        let s = tx_stage(tx_stage(tx_state_of()));
        let a = tx_abort(s);
        assert!(inv_tx(&a) && a.aborted && a.fenced && a.visible == 0);
        let dropped = tx_abort_as_is(s);
        assert!(dropped.aborted && !dropped.fenced, "AS-IS drops the fence");
        assert!(!inv_tx(&dropped), "fence-less abort is off Inv-TX");
    }

    #[test]
    fn commit_of_fully_applied_is_t1() {
        let s0 = tx_stage(tx_stage(tx_state_of()));
        let s1 = tx_apply_one(s0);
        let s2 = tx_apply_one(s1);
        let c = tx_commit(s2);
        assert!(c.committed && inv_tx(&c) && t1_holds_of(&c));
        assert_eq!(tx_recover(c), c, "committed TX is stable under recover");
        assert!(t1_modelo(c));
    }

    #[test]
    fn leftover_atom_is_the_recover_gate() {
        assert!(leftover_txn_is_aborted());
        assert!(!leftover_txn_is_aborted_as_is());
    }

    /// Live T1: two prepared keys never apply after reopen — all-or-nothing,
    /// both absent, status=abort. Model teeth: mid-apply recover vs AS-IS.
    #[test]
    fn t1_modelo_on_live_abort_reopen_is_not_ok() {
        let s0 = tx_stage(tx_stage(tx_state_of()));
        let mid = tx_apply_one(s0);
        assert!(!t1_holds_of(&mid), "mid-apply is partial");
        assert!(t1_modelo(mid));
        assert!(!t1_modelo_as_is(mid), "AS-IS leftover claims T1 of partial");
        let honest = tx_recover(mid);
        assert!(inv_tx(&honest) && honest.aborted && honest.fenced);
        let a = tx_abort(s0);
        assert!(a.aborted && a.fenced);
        assert!(!tx_abort_as_is(s0).fenced, "AS-IS abort drops the fence");
        let _ = tx_recover_as_is(mid);

        let dir = std::env::temp_dir().join(format!("pedra-t1-abort-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let txn_id;
        {
            let mut c = crate::StoreCluster::open_lab_direct(&dir, 3, 1).expect("open cluster");
            c.elect_all(80).expect("elect");
            let h = c
                .tx_start([
                    (b"t1a".as_slice(), b"v1".as_slice()),
                    (b"t1b".as_slice(), b"v2".as_slice()),
                ])
                .expect("prepare two keys");
            txn_id = h.id;
        }
        let c2 = {
            let mut c = crate::StoreCluster::open_lab_direct(&dir, 3, 1).expect("reopen");
            c.elect_all(80).expect("elect");
            c
        };
        assert_eq!(c2.count_applied_eq(b"t1a", b"v1"), 0, "key a never applied");
        assert_eq!(c2.count_applied_eq(b"t1b", b"v2"), 0, "key b never applied");
        let st = c2.get(&crate::txn_status_key(txn_id)).unwrap();
        assert_eq!(
            st.as_deref(),
            Some(b"abort".as_ref()),
            "reopen fences the leftover prepared TX as abort"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
