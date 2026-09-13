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
/-- RFC-0218 P0.3 5/6 (atom `catalog:first_install`): the first
    installation is EXACTLY the cited table — manifest commitado
    (with or without sync) proceeds; failure refuses to open (F196: the FALIDO
    never becomes the database). The AS-IS always proceeds (opens over
    installation failed — tooth planted). -/
theorem first_install_action_fate_iff :
    ∀ (out : FirstInstallOutcome) (act : FirstInstallAction),
      (first_install_action out = ok act) ↔
        ((out = FirstInstallOutcome.Committed ∧
            act = FirstInstallAction.Proceed) ∨
          (out = FirstInstallOutcome.CommittedUnsynced ∧
            act = FirstInstallAction.Proceed) ∨
          (out = FirstInstallOutcome.Failed ∧
            act = FirstInstallAction.RefuseOpen)) := by
  intro out act
  cases out with
  | Committed =>
    constructor
    · intro hval
      simp only [first_install_action] at hval
      injection hval with hv
      exact Or.inl ⟨rfl, hv.symm⟩
    · rintro (⟨-, hv⟩ | h2 | h3)
      · subst hv
        rfl
      · exact absurd h2.1 (fun h => FirstInstallOutcome.noConfusion h)
      · exact absurd h3.1 (fun h => FirstInstallOutcome.noConfusion h)
  | CommittedUnsynced =>
    constructor
    · intro hval
      simp only [first_install_action] at hval
      injection hval with hv
      exact Or.inr (Or.inl ⟨rfl, hv.symm⟩)
    · rintro (h1 | ⟨-, hv⟩ | h3)
      · exact absurd h1.1 (fun h => FirstInstallOutcome.noConfusion h)
      · subst hv
        rfl
      · exact absurd h3.1 (fun h => FirstInstallOutcome.noConfusion h)
  | Failed =>
    constructor
    · intro hval
      simp only [first_install_action] at hval
      injection hval with hv
      exact Or.inr (Or.inr ⟨rfl, hv.symm⟩)
    · rintro (h1 | h2 | ⟨-, hv⟩)
      · exact absurd h1.1 (fun h => FirstInstallOutcome.noConfusion h)
      · exact absurd h2.1 (fun h => FirstInstallOutcome.noConfusion h)
      · subst hv
        rfl

/-- RFC-0219 P0.3 (atom `catalog:bulk_manifest_persist`): the bulk
    install pays the publish of the MANIFEST inline EXACTLY when the
    default of sync of the DB pede dir-sync — sync persiste now (window
    of publish closed under the barrier of the caller, debt zeroed);
    async amortiza by debt. The AS-IS amortiza always (window of
    publish open in modo sync — tooth planted). -/
theorem bulk_manifest_persist_fate_fate_iff :
    ∀ (sync : Bool) (fate : BulkManifestFate),
      (bulk_manifest_persist_fate sync = ok fate) ↔
        ((sync = true ∧ fate = BulkManifestFate.PersistNow) ∨
          (sync = false ∧ fate = BulkManifestFate.AmortizeDebt)) := by
  intro sync fate
  simp only [bulk_manifest_persist_fate]
  split <;> rename_i c
  · constructor
    · intro hval
      injection hval with hv
      exact Or.inl ⟨c, hv.symm⟩
    · rintro (⟨-, hv⟩ | h2)
      · subst hv
        rfl
      · exact absurd h2.1 (by simp [*])
  · rw [Bool.not_eq_true] at c
    constructor
    · intro hval
      injection hval with hv
      exact Or.inr ⟨c, hv.symm⟩
    · rintro (h1 | ⟨-, hv⟩)
      · exact absurd h1.1 (by simp [*])
      · subst hv
        rfl
