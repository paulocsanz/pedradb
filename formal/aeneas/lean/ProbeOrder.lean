-- Theorems over Aeneas extract of probe_order_kernel.rs (RFC-0164).
-- Charon --start-from first_probe_on_equal_lo (walk is Iterator-refused).
import Aeneas
import ProbeOrderKernel
open Aeneas.Std Result
open pedra_aeneas_probe_order_kernel

/-- Catalog entry: equal-lo tie probes the newer table first. -/
theorem first_probe_on_equal_lo_newer :
    first_probe_on_equal_lo (1#usize) (0#usize) = ok (1#usize) := by
  unfold first_probe_on_equal_lo
  rfl

/-- AS-IS dente: equal-lo tie probes the older table first. -/
theorem first_probe_on_equal_lo_as_is_dente :
    first_probe_on_equal_lo_as_is (1#usize) (0#usize) = ok (0#usize) := by
  unfold first_probe_on_equal_lo_as_is
  rfl

/-- Packed covering: `hi` past the array is not `>= key`. -/
theorem covering_hi_ge_oob :
    covering_hi_ge ⟨[], by native_decide⟩ (0#usize) ⟨[], by native_decide⟩
      = ok false := by
  unfold covering_hi_ge
  rfl

/-- Catalog entry is a Lean `def` (index walk). Unfolds to the extracted loop. -/
theorem probe_order_covering_is_loop (nf by_lo pe his key) :
    probe_order_covering nf by_lo pe his key
      = probe_order_covering_loop nf by_lo pe his key
          (alloc.vec.Vec.with_capacity Usize (Slice.len nf)) 0#usize := by
  unfold probe_order_covering
  rfl

/-- AS-IS dente: oldest-first is the reverse-index loop. -/
theorem probe_order_covering_as_is_is_loop (nf by_lo pe his key) :
    probe_order_covering_as_is nf by_lo pe his key
      = probe_order_covering_as_is_loop nf by_lo pe his key
          (alloc.vec.Vec.with_capacity Usize (Slice.len nf)) 0#usize := by
  unfold probe_order_covering_as_is
  rfl
