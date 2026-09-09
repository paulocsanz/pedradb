-- Theorems over Aeneas extract of snapshot_kernel.rs
-- Payment is the linked rustc bodies; the former cfg(verus_keep_ghost)
-- stand-in was deleted. Fail-closed: this file must not contain a hole.
import Aeneas
import SnapshotKernel
open Aeneas Aeneas.Std Result
open pedra_aeneas_snapshot_kernel

/-- An unreserved user key is touched by the snapshot. -/
theorem snapshot_touches_user_key_unreserved :
    snapshot_touches_user_key false = ok true := by
  unfold snapshot_touches_user_key
  rfl

/-- A reserved key is left untouched by the snapshot. -/
theorem snapshot_touches_user_key_reserved_refuses :
    snapshot_touches_user_key true = ok false := by
  unfold snapshot_touches_user_key
  simp

/-- Snapshot must clear TX meta leftovers. -/
theorem snapshot_needs_txn_meta_clear_teeth :
    snapshot_needs_txn_meta_clear = ok true := by
  rfl

/-- AS-IS dente: leftover TX meta survives the snapshot. -/
theorem snapshot_needs_txn_meta_clear_as_is_dente :
    snapshot_needs_txn_meta_clear_as_is = ok false := by
  rfl
