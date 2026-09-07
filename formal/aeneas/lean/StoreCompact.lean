-- Theorems over Aeneas extract of store compact_kernel.rs
import Aeneas
import StoreCompactKernel
open Aeneas.Std Result
open pedra_aeneas_store_compact_kernel

theorem may_compact_through_zero_false :
    may_compact_through 0#u64 0#u64 1#u64 = ok false := by
  unfold may_compact_through
  rfl
