-- Theorems over Aeneas extract of vlog_gc_kernel.rs
import Aeneas
import VlogGcKernel
open Aeneas.Std Result
open pedra_aeneas_vlog_gc_kernel

theorem vlog_recover_blob_opens :
    vlog_recover_action true false false false false = ok VlogRecoverAction.OpenBlob := by
  unfold vlog_recover_action
  rfl
