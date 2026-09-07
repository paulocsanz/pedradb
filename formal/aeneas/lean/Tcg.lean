-- Theorems over Aeneas extract of tcg.rs
import Aeneas
import TcgKernel
open Aeneas.Std Result
open pedra_aeneas_tcg_kernel

theorem tcg_guest_admitted_true :
    tcg_guest_admitted true = ok true := by
  unfold tcg_guest_admitted
  rfl
