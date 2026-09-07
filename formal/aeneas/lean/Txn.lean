-- Theorems over Aeneas extract of store txn_kernel.rs (F47).
-- Ord.max.default patched to pass lt, not the Ord instance.
import Aeneas
import TxnKernel
open Aeneas.Std Result
open pedra_aeneas_txn_kernel

/-- Catalog entry: abort fence reverts, never materialises. -/
theorem txn_commit_action_abort_reverts :
    txn_commit_action true = ok TxnCommitAction.Revert := by
  unfold txn_commit_action
  rfl

/-- AS-IS dente: abort still materialises. -/
theorem txn_commit_action_as_is_dente :
    txn_commit_action_as_is true = ok TxnCommitAction.Materialise := by
  unfold txn_commit_action_as_is
  rfl
