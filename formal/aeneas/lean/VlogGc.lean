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
