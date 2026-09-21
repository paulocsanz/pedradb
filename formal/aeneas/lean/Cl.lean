-- Theorems over Aeneas extract of cl_kernel.rs
import Aeneas
import ClKernel
open Aeneas.Std Result
open pedra_aeneas_cl_kernel

theorem keep_body_without_cl_true :
    keep_body_without_cl = ok true := by
  unfold keep_body_without_cl
  rfl

theorem keep_body_without_cl_as_is_tooth :
    keep_body_without_cl_as_is = ok false := by
  unfold keep_body_without_cl_as_is
  rfl

theorem keep_body_without_cl_fate_iff :
    ∀ (r : Bool), (keep_body_without_cl = ok r) ↔ r = true := by
  intro r
  constructor
  · intro hval
    unfold keep_body_without_cl at hval
    exact (Result.ok.inj hval).symm
  · intro hr
    unfold keep_body_without_cl
    rw [hr]

theorem invalid_cl_as_zero_fate_iff :
    ∀ (r : Bool), (invalid_cl_as_zero = ok r) ↔ r = false := by
  intro r
  constructor
  · intro hval
    unfold invalid_cl_as_zero at hval
    exact (Result.ok.inj hval).symm
  · intro hr
    unfold invalid_cl_as_zero
    rw [hr]

theorem content_length_repeat_ok_fate_iff :
    ∀ (first next : Aeneas.Std.U64) (r : Bool),
      (content_length_repeat_ok first next = ok r) ↔
        r = decide (first = next) := by
  intro first next r
  constructor
  · intro hval
    unfold content_length_repeat_ok at hval
    exact (Result.ok.inj hval).symm
  · intro hr
    unfold content_length_repeat_ok
    rw [hr]

theorem short_body_vs_cl_is_error_fate_iff :
    ∀ (got declared : Aeneas.Std.U64) (r : Bool),
      (short_body_vs_cl_is_error got declared = ok r) ↔
        r = decide (got < declared) := by
  intro got declared r
  constructor
  · intro hval
    unfold short_body_vs_cl_is_error at hval
    exact (Result.ok.inj hval).symm
  · intro hr
    unfold short_body_vs_cl_is_error
    rw [hr]
