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

/-- AS-IS F59 dente: exclusive end is `0xff`. -/
theorem packed_child_end_as_is_dente :
    PACKED_CHILD_END_AS_IS = 255#u8 := by
  unfold PACKED_CHILD_END_AS_IS
  rfl
