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

/-- Exclusive start is slice `>` then unbounded end. Dual-unfold. -/
theorem user_key_in_range_excluded_unbounded (user s) :
    merge.user_key_in_range user (core.ops.range.Bound.Excluded s)
      core.ops.range.Bound.Unbounded =
      (do
        let after_start ←
          Shared1A.Insts.CoreCmpPartialOrdShared0B.gt
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) user s
        if after_start then ok true else ok false) := by
  unfold merge.user_key_in_range
  rfl

/-- Inclusive start is slice `>=` then unbounded end. Dual-unfold. -/
theorem user_key_in_range_included_unbounded (user s) :
    merge.user_key_in_range user (core.ops.range.Bound.Included s)
      core.ops.range.Bound.Unbounded =
      (do
        let after_start ←
          Shared1A.Insts.CoreCmpPartialOrdShared0B.ge
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) user s
        if after_start then ok true else ok false) := by
  unfold merge.user_key_in_range
  rfl

/-- Unbounded end never retires a stream. -/
theorem past_end_unbounded (k) :
    merge.past_end k core.ops.range.Bound.Unbounded = ok false := by
  unfold merge.past_end
  rfl

/-- Exclusive end retires at `>=` (half-open). Dual-unfold to slice `ge`. -/
theorem past_end_excluded (user e) :
    merge.past_end user (core.ops.range.Bound.Excluded e)
    = Shared1A.Insts.CoreCmpPartialOrdShared0B.ge
        (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) user e := by
  unfold merge.past_end
  rfl

/-- Inclusive end retires at `>`. Dual-unfold to slice `gt`. -/
theorem past_end_included (user e) :
    merge.past_end user (core.ops.range.Bound.Included e)
    = Shared1A.Insts.CoreCmpPartialOrdShared0B.gt
        (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) user e := by
  unfold merge.past_end
  rfl

/-- Hidden snapshot version is not emitted. -/
theorem iter_window_keep_hidden :
    merge.iter_window_keep false = ok false := by
  unfold merge.iter_window_keep
  rfl

/-- AS-IS dente: a hidden version still emits. -/
theorem iter_window_keep_as_is_dente :
    merge.iter_window_keep_as_is false = ok true := by
  unfold merge.iter_window_keep_as_is
  rfl

/-- Catalog entry: point put covers the exact start key (rustc `&[u8]`). -/
theorem write_op_covers_key_value_is_eq (start end1 user) :
    merge.write_op_covers_key key.ValueType.Value start end1 user
    = core.slice.cmp.PartialEqSlice.eq core.cmp.PartialEqU8 start user := by
  unfold merge.write_op_covers_key
  rfl

/-- Catalog entry: point delete covers the exact start key. -/
theorem write_op_covers_key_deletion_is_eq (start end1 user) :
    merge.write_op_covers_key key.ValueType.Deletion start end1 user
    = core.slice.cmp.PartialEqSlice.eq core.cmp.PartialEqU8 start user := by
  unfold merge.write_op_covers_key
  rfl

/-- Range delete covers via `range_tombstone_covers` (F30). Dual-unfold. -/
theorem write_op_covers_key_range (start end1 user) :
    merge.write_op_covers_key key.ValueType.RangeDeletion start end1 user
    = merge.range_tombstone_covers start end1 user := by
  unfold merge.write_op_covers_key
  rfl

/-- AS-IS dente: point put never conflicts. -/
theorem write_op_covers_key_as_is_value (start end1 user) :
    merge.write_op_covers_key_as_is key.ValueType.Value start end1 user
    = ok false := by
  unfold merge.write_op_covers_key_as_is
  rfl

/-- AS-IS dente: point delete never conflicts. -/
theorem write_op_covers_key_as_is_deletion (start end1 user) :
    merge.write_op_covers_key_as_is key.ValueType.Deletion start end1 user
    = ok false := by
  unfold merge.write_op_covers_key_as_is
  rfl

/-- AS-IS dente: range only hits start. Dual-unfold. -/
theorem write_op_covers_key_as_is_range (start end1 user) :
    merge.write_op_covers_key_as_is key.ValueType.RangeDeletion start end1 user
    = merge.range_tombstone_covers_as_is start end1 user := by
  unfold merge.write_op_covers_key_as_is
  rfl

/-- Catalog entry: unbounded Bound copies as unbounded (rustc `Bound`). -/
theorem bound_to_owned_unbounded :
    merge.bound_to_owned core.ops.range.Bound.Unbounded
    = ok core.ops.range.Bound.Unbounded := by
  unfold merge.bound_to_owned
  rfl

/-- Catalog entry: unbounded Bound borrows as unbounded. -/
theorem bound_as_ref_unbounded :
    merge.bound_as_ref core.ops.range.Bound.Unbounded
    = ok core.ops.range.Bound.Unbounded := by
  unfold merge.bound_as_ref
  rfl
