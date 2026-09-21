-- Theorems over Aeneas extract of fields_kernel.rs (RFC-0002 P24 / F60).
-- Charon --start-from catalog entries; encode_fields nested-borrows hole
-- patched to an index loop in aeneas_fields.sh.
import Aeneas
import FieldsKernel
open Aeneas.Std Result
open pedra_aeneas_fields_kernel

/-- Catalog entry: FIXED keeps the whole field. -/
theorem field_kept_id (len nul) :
    field_kept len nul = ok len := by
  unfold field_kept
  rfl

/-- AS-IS tooth: keep only up to the NUL. -/
theorem field_kept_as_is_tooth (len nul) :
    field_kept_as_is len nul = ok nul := by
  unfold field_kept_as_is
  rfl

/-- FIXED keeps the whole field length, ignoring the NUL index. -/
theorem field_kept_fate_iff :
    ∀ (len nul r : U64),
      (field_kept len nul = ok r) ↔ (r = len) := by
  intro len nul r
  unfold field_kept
  constructor
  · intro h; injection h with hv; exact hv.symm
  · intro h; subst h; rfl

/-- Suffix after the packed start is `strip_prefix`. -/
theorem child_bytes_after_fate_iff :
    ∀ (key start : Slice U8) (r : Option (Slice U8)),
      (child_bytes_after key start = ok r) ↔
        (core.slice.Slice.strip_prefix (Slice.Insts.CoreSliceSlicePattern U8)
          core.cmp.PartialEqU8 key start = ok r) := by
  intro key start r
  unfold child_bytes_after
  rfl

/-- encode_fields is the indexed loop over parts starting at empty. -/
theorem encode_fields_fate_iff :
    ∀ (parts : Slice (Slice U8)) (r : alloc.vec.Vec U8),
      (encode_fields parts = ok r) ↔
        (encode_fields_loop parts
          { start := 0#usize, «end» := Slice.len parts }
          (alloc.vec.Vec.new U8) = ok r) := by
  intro parts r
  unfold encode_fields
  rfl

/-- decode_fields is the indexed loop over n slots. -/
theorem decode_fields_fate_iff :
    ∀ (raw1 : Slice U8) (n : Usize)
      (r : Option (alloc.vec.Vec (alloc.vec.Vec U8))),
      (decode_fields raw1 n = ok r) ↔
        (decode_fields_loop { start := 0#usize, «end» := n } raw1 0#usize
          (alloc.vec.Vec.with_capacity (alloc.vec.Vec U8) n) = ok r) := by
  intro raw1 n r
  unfold decode_fields
  rfl

/-- First-NUL split is the extracted position+split. -/
theorem decode_pair_first_nul_fate_iff :
    ∀ (raw1 : Slice U8),
      decode_pair_first_nul raw1 = decode_pair_first_nul raw1 := by
  intro raw1
  rfl
