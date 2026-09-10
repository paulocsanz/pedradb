-- Theorems over Aeneas extract of compact_kernel.rs
import Aeneas
import CompactKernel
open Aeneas.Std Result
open pedra_aeneas_compact_kernel

theorem compact_pick_empty_noop :
    compact_pick none false false 6#u32 = ok CompactPlan.NoOp := by
  unfold compact_pick
  rfl

/-- Catalog entry: a point version is dropped exactly when a newer
    kept version exists AND it sits at or below the oldest snapshot
    (F177/F20 — a version still visible to the oldest snapshot is
    never dropped, and a version with no newer kept copy is never
    dropped either). -/
theorem point_version_fate_drop_iff_newer_kept_at_or_below_oldest_snapshot :
    ∀ (this_seq : U64) (newer_kept_seq : Option U64)
      (oldest_snapshot : U64),
      (point_version_fate this_seq newer_kept_seq oldest_snapshot
          = ok VersionFate.Drop)
        ↔ (∃ newer_seq, newer_kept_seq = some newer_seq
            ∧ newer_seq <= oldest_snapshot) := by
  intro this_seq newer_kept_seq oldest_snapshot
  unfold point_version_fate
  constructor
  · intro h
    cases newer_kept_seq with
    | none => exact absurd h (by simp)
    | some p =>
      simp at h
      exact ⟨p, rfl, h⟩
  · rintro ⟨p, hp, hc⟩
    rw [hp]
    exact if_pos hc
