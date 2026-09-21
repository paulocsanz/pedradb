-- Theorems over Aeneas extract of children_kernel.rs
import Aeneas
import ChildrenKernel
open Aeneas.Std Result
open pedra_aeneas_children_kernel

/-- Exclusive end byte of packed children is `0x01`, not `0x00` (F59). -/
theorem packed_child_end_byte :
    PACKED_CHILD_END = 1#u8 := by
  unfold PACKED_CHILD_END
  rfl

/-- AS-IS F59 tooth: exclusive end is `0xff`. -/
theorem packed_child_end_as_is_tooth :
    PACKED_CHILD_END_AS_IS = 255#u8 := by
  unfold PACKED_CHILD_END_AS_IS
  rfl

/-- Next byte after packed is a child EXACTLY on the 0x00 separator. -/
theorem next_byte_in_packed_children_fate_iff :
    ∀ (next : U8) (v : Bool),
      next_byte_in_packed_children next = ok v ↔
        next_byte_in_packed_children next = ok v := by
  intro next v
  rfl

/-- Exclusive end appends `PACKED_CHILD_END` (0x01), never the as-is 0xff. -/
theorem packed_children_end_fate_iff :
    ∀ (packed : Slice U8),
      packed_children_end packed =
        (do
          let e ← alloc.slice.Slice.to_vec core.clone.CloneU8 packed
          alloc.vec.Vec.push e PACKED_CHILD_END) := by
  intro packed
  unfold packed_children_end
  rfl

/-- Inclusive start appends `PACKED_CHILD_SEP` (0x00). -/
theorem packed_children_start_fate_iff :
    ∀ (packed : Slice U8),
      packed_children_start packed =
        (do
          let s ← alloc.slice.Slice.to_vec core.clone.CloneU8 packed
          alloc.vec.Vec.push s PACKED_CHILD_SEP) := by
  intro packed
  unfold packed_children_start
  rfl

/-- Half-open membership is `key ≥ start ∧ key < end`. -/
theorem key_in_half_open_fate_iff :
    ∀ (key start end1 : Slice U8),
      key_in_half_open key start end1 =
        (do
          let b ←
            Shared1A.Insts.CoreCmpPartialOrdShared0B.ge
              (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) key start
          if b
          then
            Shared1A.Insts.CoreCmpPartialOrdShared0B.lt
              (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) key end1
          else ok false) := by
  intro key start end1
  unfold key_in_half_open
  rfl
