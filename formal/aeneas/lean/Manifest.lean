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

/-- Catalog entry: recovery refuses to open exactly when the manifest
    observation is Corrupt, or it is Inventory while an SST listed in
    the manifest is missing on disk (F196/G8 — an absent manifest
    rescans and installs; a complete inventory serves; damage or a
    missing file never serves). -/
theorem sst_recover_action_refuse_iff_corrupt_or_inventory_missing :
    ∀ (obs : ManifestObs) (listed : ListedSst),
      (sst_recover_action obs listed = ok SstRecoverAction.RefuseOpen)
        ↔ (obs = ManifestObs.Corrupt
            ∨ (obs = ManifestObs.Inventory
                ∧ ∃ i, listed = ListedSst.Missing i)) := by
  intro obs listed
  unfold sst_recover_action
  constructor
  · intro h
    cases obs with
    | Absent => exact absurd h (by simp)
    | Corrupt => exact Or.inl rfl
    | Inventory =>
      cases listed with
      | AllPresent => exact absurd h (by simp)
      | Missing i => exact Or.inr ⟨rfl, i, rfl⟩
  · rintro (hc | ⟨hinv, i, hi⟩)
    · rw [hc]
    · rw [hinv, hi]
