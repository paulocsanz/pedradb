-- Theorems over Aeneas extract of cqe_kernel.rs
import Aeneas
import CqeKernel
open Aeneas.Std Result
open pedra_aeneas_cqe_kernel

theorem cqe_res_ok_nonneg :
    cqe_res_ok 0#i32 = ok true := by
  unfold cqe_res_ok
  simp
