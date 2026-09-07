-- Theorems over Aeneas extract of lookup_kernel.rs
import Aeneas
import LookupKernel
open Aeneas.Std Result
open pedra_aeneas_lookup_kernel

theorem snap_is_empty_zero :
    snap_is_empty 0#u64 = ok true := by
  unfold snap_is_empty
  rfl

theorem snap_is_empty_as_is_dente :
    snap_is_empty_as_is 0#u64 = ok false := by
  unfold snap_is_empty_as_is
  rfl
