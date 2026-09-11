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

/-- Catalog entry: the GcRewriteMax compaction decision is chosen
    exactly when no level holds files, GC was requested, and files
    already sit at the max level (F177/F20 — a level holding files
    yields a Merge plan, and without both GC flags the plan is NoOp). -/
theorem compact_pick_gc_rewrite_max_iff_none_and_gc_and_files_at_max :
    ∀ (lowest_level_with_files : Option U32) (files_at_max_level : Bool)
      (gc_requested : Bool) (max_level : U32),
      (compact_pick lowest_level_with_files files_at_max_level gc_requested max_level
          = ok CompactPlan.GcRewriteMax)
        ↔ (lowest_level_with_files = none ∧ gc_requested = true
            ∧ files_at_max_level = true) := by
  intro lowest_level_with_files files_at_max_level gc_requested max_level
  unfold compact_pick
  cases lowest_level_with_files with
  | none =>
    constructor
    · intro h
      cases gc_requested with
      | false => simp at h
      | true =>
        cases files_at_max_level with
        | false => simp at h
        | true => exact ⟨rfl, rfl, rfl⟩
    · rintro ⟨-, rfl, rfl⟩
      rfl
  | some l =>
    constructor
    · intro h
      simp at h
      cases hadd : l + 1#u32 with
      | ok i => rw [hadd] at h; simp at h
      | fail e => rw [hadd] at h; simp at h
      | div => rw [hadd] at h; simp at h
    · rintro ⟨habsurd, _, _⟩
      exact absurd habsurd (by simp)
