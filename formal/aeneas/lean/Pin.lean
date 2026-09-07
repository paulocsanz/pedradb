-- Theorems over Aeneas extract of pin_kernel.rs
import Aeneas
import PinKernel
open Aeneas.Std Result
open pedra_aeneas_pin_kernel

theorem may_advance_pin_forward :
    may_advance_pin 3#u64 5#u64 = ok true := by
  unfold may_advance_pin
  have h : (5#u64 > 3#u64) = true := by native_decide
  simp [h]
