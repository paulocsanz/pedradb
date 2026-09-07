-- Theorems over Aeneas extract of iter_kernel.rs
import Aeneas
import IterKernel
open Aeneas.Std Result
open pedra_aeneas_iter_kernel

theorem iter_window_keep_live :
    iter_window_keep true = ok true := by
  unfold iter_window_keep
  rfl

theorem iter_window_keep_as_is_dente :
    iter_window_keep_as_is false = ok true := by
  unfold iter_window_keep_as_is
  rfl
