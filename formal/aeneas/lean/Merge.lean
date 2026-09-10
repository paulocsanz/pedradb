-- Theorems over Aeneas extract of merge.rs visible_at (RFC-0150 / F30)
-- plus user_key_in_range / past_end. WindowKvIter is Iterator-refused.
-- RFC-0187 P1.3 / RFC-0188 P0.2: the heap-sift STRUCTURE kernel
-- (sift_step) — first `close` of the depth ladder (RFC-0188).
import Aeneas
import MergeKernel
open Aeneas.Std Result
open pedra_aeneas_merge_kernel

/-- RFC-0188 first `close` (named property, all inputs): the sift kernel
stays put EXACTLY when no repair is needed — the best child does not
beat the hole. The as-is mutant stays even when repair is needed. -/
theorem merge_sift_step_repairs_iff :
    ∀ (r_exists r_lt_l best_lt_hole : Bool),
      (merge.sift_step r_exists r_lt_l best_lt_hole
        = ok merge.SiftStep.Stay) ↔ (best_lt_hole = false) := by
  intro r_exists r_lt_l best_lt_hole
  unfold merge.sift_step
  cases r_exists <;> cases r_lt_l <;> cases best_lt_hole <;> simp

/-- The right child wins the swap EXACTLY when a repair is needed, the
right child exists, and it beats the left child. -/
theorem merge_sift_step_swap_right_iff :
    ∀ (r_exists r_lt_l best_lt_hole : Bool),
      (merge.sift_step r_exists r_lt_l best_lt_hole
        = ok merge.SiftStep.SwapRight)
        ↔ (best_lt_hole = true ∧ r_exists = true ∧ r_lt_l = true) := by
  intro r_exists r_lt_l best_lt_hole
  unfold merge.sift_step
  cases r_exists <;> cases r_lt_l <;> cases best_lt_hole <;> simp

/-- AS-IS dente (Lean side): on every repairing input the mutant stays
and the kernel does not — the decisions diverge. -/
theorem merge_sift_step_as_is_diverges_on_repair (r_exists r_lt_l : Bool) :
    merge.sift_step_as_is r_exists r_lt_l true
      ≠ merge.sift_step r_exists r_lt_l true := by
  unfold merge.sift_step merge.sift_step_as_is
  cases r_exists <;> cases r_lt_l <;> simp

/-- Catalog entry: a deletion is never live. -/
theorem visible_at_deletion :
    merge.visible_at key.ValueType.Deletion false = ok false := by
  unfold merge.visible_at
  rfl

/-- RFC-0188 P1.6 first `atom` (data-fate rule, all inputs): a deletion
never surfaces live, WHATEVER the covering-range says — the fate of the
deleted version is decided by the kind alone. -/
theorem visible_at_deletion_never_live :
    ∀ (range_hidden : Bool),
      merge.visible_at key.ValueType.Deletion range_hidden = ok false := by
  intro range_hidden
  unfold merge.visible_at
  cases range_hidden <;> rfl

/-- RFC-0191 P0.2 product corollary R1-deletion: a Deletion never
surfaces live. Unfolds production `visible_at` (the get-path atom). -/
theorem r1_deletion_never_live :
    ∀ (range_hidden : Bool),
      merge.visible_at key.ValueType.Deletion range_hidden = ok false := by
  intro range_hidden
  unfold merge.visible_at
  cases range_hidden <;> rfl

/-- RFC-0191 P1.1 product corollary R1 both arms: deletion never live,
Value live iff not hidden. Unfolds production `visible_at` twice. -/
theorem r1_get_atom :
    ∀ (range_hidden : Bool),
      (merge.visible_at key.ValueType.Deletion range_hidden = ok false)
      ∧ (merge.visible_at key.ValueType.Value range_hidden
          = ok (!range_hidden)) := by
  intro range_hidden
  constructor
  · unfold merge.visible_at; cases range_hidden <;> rfl
  · unfold merge.visible_at; cases range_hidden <;> rfl

/-- A Value surfaces exactly when no covering range hides it (all inputs). -/
theorem visible_at_value_live_iff_not_hidden :
    ∀ (range_hidden : Bool),
      merge.visible_at key.ValueType.Value range_hidden = ok (!range_hidden) := by
  intro range_hidden
  unfold merge.visible_at
  cases range_hidden <;> rfl

/-- Catalog entry: a Value is live unless a covering range hides it. -/
theorem visible_at_value_live :
    merge.visible_at key.ValueType.Value false = ok true := by
  unfold merge.visible_at
  rfl

/-- Catalog entry: a Value hidden by a covering range is not live. -/
theorem visible_at_value_hidden :
    merge.visible_at key.ValueType.Value true = ok false := by
  unfold merge.visible_at
  rfl

/-- Catalog entry: a range deletion is never live. -/
theorem visible_at_range_deletion :
    merge.visible_at key.ValueType.RangeDeletion false = ok false := by
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

/-- Exclusive end, unbounded start: key must be `<` end. Dual-unfold. -/
theorem user_key_in_range_unbounded_excluded_end (user e) :
    merge.user_key_in_range user core.ops.range.Bound.Unbounded
      (core.ops.range.Bound.Excluded e) =
      (do
        let before_end ←
          Shared1A.Insts.CoreCmpPartialOrdShared0B.lt
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) user e
        if true then ok before_end else ok false) := by
  unfold merge.user_key_in_range
  rfl

/-- Inclusive end, unbounded start: key must be `<=` end. Dual-unfold. -/
theorem user_key_in_range_unbounded_included_end (user e) :
    merge.user_key_in_range user core.ops.range.Bound.Unbounded
      (core.ops.range.Bound.Included e) =
      (do
        let before_end ←
          Shared1A.Insts.CoreCmpPartialOrdShared0B.le
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) user e
        if true then ok before_end else ok false) := by
  unfold merge.user_key_in_range
  rfl

/-- Included start + included end: key `>=` s and key `<=` e. Dual-unfold. -/
theorem user_key_in_range_included_start_included_end
    (user s e : Slice U8) :
    merge.user_key_in_range user
      (core.ops.range.Bound.Included s)
      (core.ops.range.Bound.Included e) =
      (do
        let after_start ←
          Shared1A.Insts.CoreCmpPartialOrdShared0B.ge
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) user s
        let before_end ←
          Shared1A.Insts.CoreCmpPartialOrdShared0B.le
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) user e
        if after_start then ok before_end else ok false) := by
  unfold merge.user_key_in_range
  rfl

/-- Excluded start + excluded end: key `>` s and key `<` e. Dual-unfold. -/
theorem user_key_in_range_excluded_start_excluded_end
    (user s e : Slice U8) :
    merge.user_key_in_range user
      (core.ops.range.Bound.Excluded s)
      (core.ops.range.Bound.Excluded e) =
      (do
        let after_start ←
          Shared1A.Insts.CoreCmpPartialOrdShared0B.gt
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) user s
        let before_end ←
          Shared1A.Insts.CoreCmpPartialOrdShared0B.lt
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) user e
        if after_start then ok before_end else ok false) := by
  unfold merge.user_key_in_range
  rfl

/-- Included start + excluded end: key `>=` s and key `<` e. Dual-unfold. -/
theorem user_key_in_range_included_start_excluded_end
    (user s e : Slice U8) :
    merge.user_key_in_range user
      (core.ops.range.Bound.Included s)
      (core.ops.range.Bound.Excluded e) =
      (do
        let after_start ←
          Shared1A.Insts.CoreCmpPartialOrdShared0B.ge
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) user s
        let before_end ←
          Shared1A.Insts.CoreCmpPartialOrdShared0B.lt
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) user e
        if after_start then ok before_end else ok false) := by
  unfold merge.user_key_in_range
  rfl

/-- Excluded start + included end: key `>` s and key `<=` e. Dual-unfold. -/
theorem user_key_in_range_excluded_start_included_end
    (user s e : Slice U8) :
    merge.user_key_in_range user
      (core.ops.range.Bound.Excluded s)
      (core.ops.range.Bound.Included e) =
      (do
        let after_start ←
          Shared1A.Insts.CoreCmpPartialOrdShared0B.gt
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) user s
        let before_end ←
          Shared1A.Insts.CoreCmpPartialOrdShared0B.le
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) user e
        if after_start then ok before_end else ok false) := by
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

/-- Catalog entry: rustc `&[u8]` cover is `>= start` then `< end` — not the u64 cartoon. -/
theorem range_tombstone_covers_is_ge_then_lt (start end1 user) :
    merge.range_tombstone_covers start end1 user =
      (do
        let b ←
          Shared1A.Insts.CoreCmpPartialOrdShared0B.ge
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) user start
        if b then
          Shared1A.Insts.CoreCmpPartialOrdShared0B.lt
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) user end1
        else ok false) := by
  unfold merge.range_tombstone_covers
  rfl

/-- AS-IS F30: rustc cover is start-key equality (misses interior). -/
theorem range_tombstone_covers_as_is_is_eq (start end1 user) :
    merge.range_tombstone_covers_as_is start end1 user
    = core.slice.cmp.PartialEqSlice.eq core.cmp.PartialEqU8 user start := by
  unfold merge.range_tombstone_covers_as_is
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

/-- Included Bound borrows via `Bytes::as_ref`. Dual-unfold. -/
theorem bound_as_ref_included (s) :
    merge.bound_as_ref (core.ops.range.Bound.Included s) =
      (do
        let s1 ← bytes.bytes.Bytes.Insts.CoreConvertAsRefSliceU8.as_ref s
        ok (core.ops.range.Bound.Included s1)) := by
  unfold merge.bound_as_ref
  rfl

/-- Excluded Bound borrows via `Bytes::as_ref`. Dual-unfold. -/
theorem bound_as_ref_excluded (s) :
    merge.bound_as_ref (core.ops.range.Bound.Excluded s) =
      (do
        let s1 ← bytes.bytes.Bytes.Insts.CoreConvertAsRefSliceU8.as_ref s
        ok (core.ops.range.Bound.Excluded s1)) := by
  unfold merge.bound_as_ref
  rfl

/-- Included Bound copies via `Bytes::copy_from_slice`. Dual-unfold. -/
theorem bound_to_owned_included (s) :
    merge.bound_to_owned (core.ops.range.Bound.Included s) =
      (do
        let b1 ← bytes.bytes.Bytes.copy_from_slice s
        ok (core.ops.range.Bound.Included b1)) := by
  unfold merge.bound_to_owned
  rfl

/-- Excluded Bound copies via `Bytes::copy_from_slice`. Dual-unfold. -/
theorem bound_to_owned_excluded (s) :
    merge.bound_to_owned (core.ops.range.Bound.Excluded s) =
      (do
        let b1 ← bytes.bytes.Bytes.copy_from_slice s
        ok (core.ops.range.Bound.Excluded b1)) := by
  unfold merge.bound_to_owned
  rfl
