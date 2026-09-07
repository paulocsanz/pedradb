-- Theorems over Aeneas extract of store apply_kernel.rs (RFC-0152 F10-apply).
import Aeneas
import StoreApplyKernel
open Aeneas.Std Result
open pedra_aeneas_store_apply_kernel

/-- Catalog clone: a log hole stops without advancing. -/
theorem apply_advance_hole_stops :
    apply_advance (1#u64) (2#u64) false = ok ApplyAction.Stop := by
  unfold apply_advance
  rfl

/-- AS-IS dente: a hole still applies. -/
theorem apply_advance_as_is_dente :
    apply_advance_as_is_skip_holes (1#u64) (2#u64) false = ok ApplyAction.Apply := by
  unfold apply_advance_as_is_skip_holes
  rfl
