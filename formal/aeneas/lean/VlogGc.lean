-- Theorems over Aeneas extract of vlog_gc_kernel.rs
import Aeneas
import VlogGcKernel
open Aeneas.Std Result
open pedra_aeneas_vlog_gc_kernel

theorem vlog_recover_blob_opens :
    vlog_recover_action true false false false false = ok VlogRecoverAction.OpenBlob := by
  unfold vlog_recover_action
  rfl

/-- Catalog entry: the blob GC rewrites exactly when the gen is
    inactive AND still carries bytes (F-active-gen — an active gen is
    never rewritten; an empty inactive gen has nothing to rewrite). -/
theorem blob_gc_action_rewrite_iff_inactive_with_bytes :
    ∀ (is_active : Bool) (bytes : U64),
      (blob_gc_action is_active bytes = ok BlobGcAction.Rewrite)
        ↔ (is_active = false ∧ bytes > 0#u64) := by
  intro is_active bytes
  unfold blob_gc_action
  constructor
  · intro h
    split at h
    · next c1 => exact absurd h (by simp)
    · next c1 =>
      split at h
      · next c2 => exact ⟨by simp at c1; exact c1, c2⟩
      · next c2 => exact absurd h (by simp)
  · rintro ⟨h1, h2⟩
    rw [if_neg (by simp [h1]), if_pos h2]

/-- Catalog entry: recovery refuses to open exactly when the MANIFEST
    says the swing committed (use_new) yet neither file is on disk —
    blob mode is off, large values are wanted, no primary, no new
    (F51/G-swing: inventing an empty primary would make every large
    value vanish). -/
theorem vlog_recover_action_refuse_open_iff_wants_large_use_new_and_nothing_on_disk :
    ∀ (blob_active wants_large primary_exists use_new new_exists : Bool),
      (vlog_recover_action blob_active wants_large primary_exists use_new new_exists
          = ok VlogRecoverAction.RefuseOpen)
        ↔ (blob_active = false ∧ wants_large = true ∧ primary_exists = false
            ∧ use_new = true ∧ new_exists = false) := by
  intro blob_active wants_large primary_exists use_new new_exists
  unfold vlog_recover_action
  cases blob_active <;> cases wants_large <;> cases primary_exists <;>
    cases use_new <;> cases new_exists <;> simp
