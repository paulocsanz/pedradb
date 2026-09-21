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

/-- AS-IS tooth: CasFailed freezes. -/
theorem dcs_apply_should_advance_as_is_tooth :
    apply_kernel.dcs_apply_should_advance_as_is false true = ok false := by
  unfold apply_kernel.dcs_apply_should_advance_as_is
  rfl

/-- Catalog entry (RFC-0218 P2.2, atom `dcs_apply`): the advance
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

/-- Catalog entry (RFC-0218 P2.2, atom `dcs_advance_bool`): the
    advance bit is exactly the cited ite — ok1 forces true; a failed
    result advances exactly when it failed on Cas (F12/F22). -/
theorem dcs_apply_should_advance_fate_iff :
    ∀ (ok1 cas_failed : Bool) (v : Bool),
      (apply_kernel.dcs_apply_should_advance ok1 cas_failed = ok v) ↔
        (if ok1 = true then v = true else v = cas_failed) := by
  intro ok1 cas_failed v
  cases ok1 with
  | true =>
    rw [if_pos (by simp : (true : Bool) = true)]
    constructor
    · intro h
      have h' : ok true = ok v := h
      injection h' with hv
      exact hv.symm
    · intro h
      show ok true = ok v
      rw [h]
  | false =>
    rw [if_neg (by simp : ¬((false : Bool) = true))]
    constructor
    · intro h
      have h' : ok cas_failed = ok v := h
      injection h' with hv
      exact hv.symm
    · intro h
      show ok cas_failed = ok v
      rw [h]
