-- Theorems over Aeneas extract of manifest_kernel.rs
import Aeneas
import ManifestKernel
open Aeneas.Std Result
open pedra_aeneas_manifest_kernel

theorem sst_recover_absent_scans :
    sst_recover_action ManifestObs.Absent ListedSst.AllPresent
      = ok SstRecoverAction.ScanAndInstall := by
  unfold sst_recover_action
  rfl
