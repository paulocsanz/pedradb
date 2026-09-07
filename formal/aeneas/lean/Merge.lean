-- Theorems over Aeneas extract of merge.rs visible_at (RFC-0150 / F30)
-- plus user_key_in_range / past_end. WindowKvIter is Iterator-refused.
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

/-- Unbounded window contains every key. -/
theorem user_key_in_range_unbounded (k) :
    merge.user_key_in_range k core.ops.range.Bound.Unbounded
      core.ops.range.Bound.Unbounded = ok true := by
  unfold merge.user_key_in_range
  rfl

/-- Unbounded end never retires a stream. -/
theorem past_end_unbounded (k) :
    merge.past_end k core.ops.range.Bound.Unbounded = ok false := by
  unfold merge.past_end
  rfl
