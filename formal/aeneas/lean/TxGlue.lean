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

/-- AS-IS dente: a committed failed TX only reverts locally — the
    majority keeps the poisoned entry. -/
theorem tx_range_as_is_local_only_dente :
    tx_range_action_as_is_local_only true true = ok TxRangeAction.LocalRevert := by
  unfold tx_range_action_as_is_local_only
  rfl
