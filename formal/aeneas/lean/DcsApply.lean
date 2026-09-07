-- Theorems over Aeneas extract of dcs apply_kernel.rs (F12/F22).
import Aeneas
import DcsApplyKernel
open Aeneas.Std Result
open pedra_aeneas_dcs_apply_kernel

/-- Catalog entry: CasFailed still advances. -/
theorem dcs_apply_should_advance_cas :
    apply_kernel.dcs_apply_should_advance false true = ok true := by
  unfold apply_kernel.dcs_apply_should_advance
  rfl

/-- AS-IS dente: CasFailed freezes. -/
theorem dcs_apply_should_advance_as_is_dente :
    apply_kernel.dcs_apply_should_advance_as_is false true = ok false := by
  unfold apply_kernel.dcs_apply_should_advance_as_is
  rfl
