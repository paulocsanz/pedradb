-- Theorems over Aeneas extract of snapshot_kernel.rs
import Aeneas
import SnapshotKernel
open Aeneas.Std Result
open pedra_aeneas_snapshot_kernel

theorem snapshot_touches_user_key_unreserved :
    snapshot_touches_user_key false = ok true := by
  unfold snapshot_touches_user_key
  rfl
