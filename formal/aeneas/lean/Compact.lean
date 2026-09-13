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

/-- Catalog entry: the GC oldest-boundary is exactly the live pin
    when one exists; unpinned, it is the visible minimum of last and
    visible seq — GC never advances past a pin nor past what is
    visible (RFC-0150 P2b/F20; the min is the Aeneas Ord boundary,
    stated over its Result). -/
theorem gc_oldest_from_pin_value_iff_pin_or_unpinned_visible_min :
    ∀ (oldest_pin : Option U64) (last_seq : U64) (visible_seq : U64) (v : U64),
      (gc_oldest_from_pin oldest_pin last_seq visible_seq = ok v)
        ↔ (oldest_pin = some v
            ∨ (oldest_pin = none ∧
                core.cmp.Ord.min.trait_default core.cmp.OrdU64 last_seq visible_seq
                  = ok v)) := by
  intro oldest_pin last_seq visible_seq v
  unfold gc_oldest_from_pin
  cases oldest_pin <;> simp

private theorem bind_ok_inv {α β} (x : Result α) (f : α → Result β) (v : β)
    (h : Aeneas.Std.bind x f = ok v) : ∃ a, x = ok a ∧ f a = ok v := by
  cases x with
  | ok a => exact ⟨a, rfl, h⟩
  | fail e => exact absurd h (by simp)
  | div => exact absurd h (by simp)

/-- An ok chain reassembles into an ok bind. -/
private theorem bind_intro {α β} {x : Result α} {f : α → Result β} {v : β}
    (a : α) (hx : x = ok a) (h : f a = ok v) : Aeneas.Std.bind x f = ok v := by
  rw [hx]
  exact h

/-- RFC-0218 P1.1 5/10 (atom `catalog:compact_split`, entry
    `compact_should_split`): split is EXACTLY the compare against the
    cited target COMPACT_TARGET_FILE_BYTES (cited bind: the gate
    produces the index and the comparison decides). The AS-IS never splits
    (ok false — output file without borne; tooth planted). -/
theorem compact_should_split_fate_iff :
    ∀ (w : U64) (v : Bool),
      (compact_should_split w = ok v) ↔
      (∃ i, COMPACT_TARGET_FILE_BYTES = ok i ∧
            compact_should_split_at w i = ok v) := by
  intro w v
  constructor
  · intro hval
    unfold compact_should_split at hval
    exact bind_ok_inv _ _ _ hval
  · rintro ⟨i, hT, hs⟩
    unfold compact_should_split
    exact bind_intro i hT hs

/-- RFC-0218 P1.1 6/10 (atom `catalog:compact_split_at`, entry
    `compact_should_split_at`): split-in the-point is EXACTLY the lift
    cited `written_bytes >= target` (decides). The AS-IS is the constante
    false (mutants never dividem — tooth planted). -/
theorem compact_should_split_at_fate_iff :
    ∀ (w : U64) (t : U64) (v : Bool),
      (compact_should_split_at w t = ok v) ↔ (v = decide (w >= t)) := by
  intro w t v
  constructor
  · intro hval
    unfold compact_should_split_at at hval
    injection hval with hv
    exact hv.symm
  · rintro hv
    subst hv
    rfl

/-- RFC-0218 P1.1 7/10 (atom `catalog:lone_tombstone`, entry
    `lone_tombstone_fate`): the lone tombstone falls ONLY in the lowest level
— Drop requires bottommost AND single-newest; all the rest stays
    Keep. The AS-IS ignores bottommost (Drop outside the bottom — tooth
    planted). -/
theorem lone_tombstone_fate_iff :
    ∀ (bottommost : Bool) (lone_newest : Bool) (r : VersionFate),
      (lone_tombstone_fate bottommost lone_newest = ok r) ↔
      ((bottommost = true ∧ lone_newest = true ∧ r = VersionFate.Drop) ∨
       (bottommost = true ∧ lone_newest = false ∧ r = VersionFate.Keep) ∨
       (bottommost = false ∧ r = VersionFate.Keep)) := by
  intro bottommost lone_newest r
  constructor
  · intro hval
    unfold lone_tombstone_fate at hval
    split at hval
    · next hb =>
      split at hval
      · next hl =>
        injection hval with hv
        exact Or.inl ⟨hb, hl, hv.symm⟩
      · next hl =>
        simp only [Bool.not_eq_true] at hl
        injection hval with hv
        exact Or.inr (Or.inl ⟨hb, hl, hv.symm⟩)
    · next hb =>
      simp only [Bool.not_eq_true] at hb
      injection hval with hv
      exact Or.inr (Or.inr ⟨hb, hv.symm⟩)
  · rintro (⟨hb, hl, hv⟩ | ⟨hb, hl, hv⟩ | ⟨hb, hv⟩)
    · subst hb; subst hl; subst hv; rfl
    · subst hb; subst hl; subst hv; rfl
    · subst hb; subst hv; rfl
