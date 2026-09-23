-- Theorems over production filter_partition_kernel.rs (RFC-0236 P0.1).
import Aeneas
import FilterPartitionKernel
open Aeneas.Std Result
open pedra_aeneas_filter_partition_kernel

/-- RFC-0236 P0.1 (atom `catalog:filter_partition`): nparts ≤ 1
    collapses every key to partition 0 (one filter). AS-IS is always 0. -/
theorem filter_partition_collapses_iff :
    ∀ (h1 : U64) (nparts : U32),
      (nparts ≤ 1#u32) →
      filter_partition h1 nparts = ok 0#u32 := by
  intro h1 nparts h
  unfold filter_partition
  simp [h]

/-- AS-IS dente: every key still partition 0. -/
theorem filter_partition_as_is_always_zero :
    ∀ (h1 : U64) (nparts : U32),
      filter_partition_as_is h1 nparts = ok 0#u32 := by
  intro h1 nparts
  unfold filter_partition_as_is
  rfl
