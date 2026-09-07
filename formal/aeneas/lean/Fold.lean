-- Theorems over Aeneas extract of fold_kernel.rs
import Aeneas
import FoldKernel
open Aeneas.Std Result
open pedra_aeneas_fold_kernel

theorem fold_event_hides_key_is_def : True := by
  have _ := @fold_event_hides_key
  trivial
