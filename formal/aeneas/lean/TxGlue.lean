-- Theorems over Aeneas extract of tx_glue_kernel.rs
-- Payment is the linked rustc bodies; the former cfg(verus_keep_ghost)
-- stand-in was deleted. Fail-closed: this file must not contain a hole.
import Aeneas
import TxGlueKernel
open Aeneas Aeneas.Std Result
open pedra_aeneas_tx_glue_kernel

/-- Committed range with a failed TX: the majority must revert it. -/
theorem tx_range_failed_committed_majority_reverts :
    tx_range_action true true = ok TxRangeAction.MajorityRevert := by
  unfold tx_range_action
  rfl

/-- Uncommitted range with a failed TX: local revert only. -/
theorem tx_range_failed_uncommitted_local_reverts :
    tx_range_action false true = ok TxRangeAction.LocalRevert := by
  unfold tx_range_action
  rfl

/-- A live TX keeps the committed range. -/
theorem tx_range_keep_committed :
    tx_range_action true false = ok TxRangeAction.KeepCommitted := by
  unfold tx_range_action
  rfl

/-- AS-IS tooth: a committed failed TX only reverts locally — the
    majority keeps the poisoned entry. -/
theorem tx_range_as_is_local_only_tooth :
    tx_range_action_as_is_local_only true true = ok TxRangeAction.LocalRevert := by
  unfold tx_range_action_as_is_local_only
  rfl

/-- Catalog entry: the majority must revert exactly when the TX failed
    AND the range committed (F47/F34 — forall, subsumes the fixed-input
    theorems above). -/
theorem tx_range_action_majority_reverts_iff_failed_and_committed :
    ∀ (range_committed : Bool) (tx_failed : Bool),
      (tx_range_action range_committed tx_failed
          = ok TxRangeAction.MajorityRevert)
        ↔ (tx_failed = true ∧ range_committed = true) := by
  intro range_committed tx_failed
  unfold tx_range_action
  constructor
  · intro h
    split at h
    · next c1 =>
      split at h
      · next c2 => exact ⟨c1, c2⟩
      · next c2 => exact absurd h (by simp)
    · next c1 => exact absurd h (by simp)
  · rintro ⟨hd, hc⟩
    rw [if_pos hd, if_pos hc]
