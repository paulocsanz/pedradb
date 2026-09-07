-- Theorems over Aeneas extract of compact_kernel.rs
import Aeneas
import CompactKernel
open Aeneas.Std Result
open pedra_aeneas_compact_kernel

theorem compact_pick_empty_noop :
    compact_pick none false false 6#u32 = ok CompactPlan.NoOp := by
  unfold compact_pick
  rfl
