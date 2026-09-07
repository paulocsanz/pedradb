-- Theorems over Aeneas extract of changelog_kernel.rs
import Aeneas
import ChangelogKernel
open Aeneas.Std Result
open pedra_aeneas_changelog_kernel

theorem changelog_should_store_due :
    changelog_should_store 5#u64 3#u64 = ok true := by
  unfold changelog_should_store
  have hgt : (3#u64 > 0#u64) = true := by native_decide
  have hge : (5#u64 ≥ 3#u64) = true := by native_decide
  simp [hgt, hge]
