-- Theorems over Aeneas extract of merge.rs visible_at (RFC-0150 / F30).
-- Charon --start-from visible_at (WindowKvIter is Iterator-refused).
import Aeneas
import MergeKernel
open Aeneas.Std Result
open pedra_aeneas_merge_kernel

/-- Catalog entry: a deletion is never live. -/
theorem visible_at_deletion :
    merge.visible_at key.ValueType.Deletion false = ok false := by
  unfold merge.visible_at
  rfl

/-- AS-IS dente: a deletion still scans live. -/
theorem visible_at_as_is_dente :
    merge.visible_at_as_is key.ValueType.Deletion true = ok true := by
  unfold merge.visible_at_as_is
  rfl
