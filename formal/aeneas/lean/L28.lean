-- Theorems over Aeneas extract of l28.rs
import Aeneas
import L28Kernel
open Aeneas.Std Result
open pedra_aeneas_l28_kernel

theorem l28_durability_all_ok :
    l28_durability_ok true true true = ok true := by
  unfold l28_durability_ok
  rfl
