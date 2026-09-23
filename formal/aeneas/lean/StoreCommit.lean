-- Theorems over Aeneas extract of store commit_kernel.rs (F10/F23).
import Aeneas
import StoreCommitKernel
open Aeneas.Std Result
open pedra_aeneas_store_commit_kernel

/-- Majority of a current-term index may commit. -/
theorem may_commit_at_current_majority :
    may_commit_at (3#u64) (3#u64) true = ok true := by
  unfold may_commit_at
  rfl

/-- AS-IS dente: majority of a previous-term index still commits. -/
theorem may_commit_at_as_is_dente :
    may_commit_at_as_is (1#u64) (2#u64) true = ok true := by
  unfold may_commit_at_as_is
  rfl
