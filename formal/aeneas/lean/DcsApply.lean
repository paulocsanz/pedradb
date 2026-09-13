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

/-- Catalog entry (RFC-0218 P2.2, átomo `dcs_apply`): the advance
    decision on a DCS apply result is exactly the cited tree — ok
    advances; CasFailed still advances; Core/LeaseNotFound/Corrupt
    freeze (F12/F22). -/
theorem dcs_apply_should_advance_result_fate_iff :
    ∀ {T : Type} (r : core.result.Result T DcsError) (v : Bool),
      (apply_kernel.dcs_apply_should_advance_result r = ok v) ↔
        (match r with
         | core.result.Result.Ok _ => v = true
         | core.result.Result.Err DcsError.Core => v = false
         | core.result.Result.Err (DcsError.CasFailed _) => v = true
         | core.result.Result.Err (DcsError.LeaseNotFound _) => v = false
         | core.result.Result.Err (DcsError.Corrupt _) => v = false) := by
  intro T r v
  cases r with
  | Ok a =>
    first | dsimp only | skip
    constructor
    · intro h
      have h' : ok true = ok v := h
      injection h' with hv
      exact hv.symm
    · intro h
      show ok true = ok v
      rw [h]
  | Err de =>
    cases de with
    | Core =>
      first | dsimp only | skip
      constructor
      · intro h
        have h' : ok false = ok v := h
        injection h' with hv
        exact hv.symm
      · intro h
        show ok false = ok v
        rw [h]
    | CasFailed s =>
      first | dsimp only | skip
      constructor
      · intro h
        have h' : ok true = ok v := h
        injection h' with hv
        exact hv.symm
      · intro h
        show ok true = ok v
        rw [h]
    | LeaseNotFound n =>
      first | dsimp only | skip
      constructor
      · intro h
        have h' : ok false = ok v := h
        injection h' with hv
        exact hv.symm
      · intro h
        show ok false = ok v
        rw [h]
    | Corrupt s =>
      first | dsimp only | skip
      constructor
      · intro h
        have h' : ok false = ok v := h
        injection h' with hv
        exact hv.symm
      · intro h
        show ok false = ok v
        rw [h]
