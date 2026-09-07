-- Theorems over Aeneas extract of cl_kernel.rs
import Aeneas
import ClKernel
open Aeneas.Std Result
open pedra_aeneas_cl_kernel

theorem keep_body_without_cl_true :
    keep_body_without_cl = ok true := by
  unfold keep_body_without_cl
  rfl

theorem keep_body_without_cl_as_is_dente :
    keep_body_without_cl_as_is = ok false := by
  unfold keep_body_without_cl_as_is
  rfl
