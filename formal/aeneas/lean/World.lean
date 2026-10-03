-- Theorems over Aeneas extract of world_kernel.rs (RFC-0059 trajectory).
-- Option<&'static str> and HashMap fold holes patched in aeneas_world.sh.
import Aeneas
import WorldKernel
open Aeneas.Std Result
open pedra_aeneas_world_kernel

def sample (term snap applied : U64) : TrajectorySample :=
  {
    step := 0#u32,
    node := 1#u64,
    range := 1#u64,
    term,
    snapshot_index := snap,
    applied_index := applied
  }

/-- Catalog entry: applied watermark regression is reported. -/
theorem trajectory_violation_applied :
    trajectory_violation (sample (1#u64) (4#u64) (7#u64))
      (sample (1#u64) (4#u64) (3#u64)) =
      ok (some (toStr "applied_index")) := by
  unfold trajectory_violation sample
  simp

/-- AS-IS dente: applied regression is blessed. -/
theorem trajectory_violation_as_is_dente :
    trajectory_violation_as_is (sample (1#u64) (4#u64) (7#u64))
      (sample (1#u64) (4#u64) (3#u64)) =
      ok none := by
  unfold trajectory_violation_as_is sample
  simp

/-- Watermark regression is reported: term, then snapshot, then applied. -/
theorem trajectory_violation_fate_iff :
    ∀ (prev cur : TrajectorySample) (r : Option Str),
      (trajectory_violation prev cur = ok r) ↔
        ((cur.term < prev.term ∧ r = some (toStr "term")) ∨
          (¬(cur.term < prev.term) ∧ cur.snapshot_index < prev.snapshot_index ∧
            r = some (toStr "snapshot_index")) ∨
          (¬(cur.term < prev.term) ∧
            ¬(cur.snapshot_index < prev.snapshot_index) ∧
            cur.applied_index < prev.applied_index ∧
            r = some (toStr "applied_index")) ∨
          (¬(cur.term < prev.term) ∧
            ¬(cur.snapshot_index < prev.snapshot_index) ∧
            ¬(cur.applied_index < prev.applied_index) ∧ r = none)) := by
  intro prev cur r
  unfold trajectory_violation
  split
  · next ht =>
    constructor
    · intro h; injection h with hv; exact Or.inl ⟨ht, hv.symm⟩
    · rintro (⟨_, hv⟩ | ⟨hnt, _⟩ | ⟨hnt, _⟩ | ⟨hnt, _⟩)
      · subst hv; rfl
      · exact absurd ht hnt
      · exact absurd ht hnt
      · exact absurd ht hnt
  · next ht =>
    split
    · next hs =>
      constructor
      · intro h; injection h with hv
        exact Or.inr (Or.inl ⟨ht, hs, hv.symm⟩)
      · rintro (⟨ht', _⟩ | ⟨_, _, hv⟩ | ⟨_, hns, _⟩ | ⟨_, hns, _⟩)
        · exact absurd ht' ht
        · subst hv; rfl
        · exact absurd hs hns
        · exact absurd hs hns
    · next hs =>
      split
      · next ha =>
        constructor
        · intro h; injection h with hv
          exact Or.inr (Or.inr (Or.inl ⟨ht, hs, ha, hv.symm⟩))
        · rintro (⟨ht', _⟩ | ⟨_, hs', _⟩ | ⟨_, _, _, hv⟩ | ⟨_, _, hna, _⟩)
          · exact absurd ht' ht
          · exact absurd hs' hs
          · subst hv; rfl
          · exact absurd ha hna
      · next ha =>
        constructor
        · intro h; injection h with hv
          exact Or.inr (Or.inr (Or.inr ⟨ht, hs, ha, hv.symm⟩))
        · rintro (⟨ht', _⟩ | ⟨_, hs', _⟩ | ⟨_, _, ha', _⟩ | ⟨_, _, _, hv⟩)
          · exact absurd ht' ht
          · exact absurd hs' hs
          · exact absurd ha' ha
          · subst hv; rfl

/-- Fold over samples is the extracted loop from empty. -/
theorem check_trajectory_fate_iff :
    ∀ (samples : Slice TrajectorySample) (r : alloc.vec.Vec String),
      (check_trajectory samples = ok r) ↔
        (check_trajectory_loop samples (alloc.vec.Vec.new String) 0#usize
          = ok r) := by
  intro samples r
  unfold check_trajectory
  rfl
