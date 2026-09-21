-- Theorems over production workload_class_kernel.rs (RFC-0235 P0.1).
import Aeneas
import WorkloadClassKernel
open Aeneas.Std Result
open pedra_aeneas_workload_class_kernel

/-- RFC-0235 P0.1 (atom `catalog:workload_class`): when q is the unique
    strict max of (z0,z1,q,W) the class is ShortRange. Fate forall over
    the body. AS-IS is always Mixed. -/
theorem workload_class_short_range_iff :
    ∀ (z0 z1 q w : U64),
      (q > z0 ∧ q > z1 ∧ q > w) →
      workload_class z0 z1 q w = ok WorkloadClass.ShortRange := by
  intro z0 z1 q w h
  unfold workload_class
  cases h with
  | intro h0 hrest =>
    cases hrest with
    | intro h1 hw =>
      simp [h0, h1, hw]

/-- AS-IS tooth: every mix is Mixed. -/
theorem workload_class_as_is_always_mixed :
    ∀ (z0 z1 q w : U64),
      workload_class_as_is z0 z1 q w = ok WorkloadClass.Mixed := by
  intro z0 z1 q w
  unfold workload_class_as_is
  rfl
