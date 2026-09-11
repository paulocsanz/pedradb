-- Theorems over Aeneas extract of leveling.rs (leveled compaction).
-- RUSTFLAGS=--cfg test so as_is mutants are visible; pick Iterator holes
-- patched to index loops in aeneas_leveling.sh.
import Aeneas
import LevelingKernel
open Aeneas.Std Result
open pedra_aeneas_leveling_kernel

/-- Catalog entry: L0 has no size target. -/
theorem level_target_bytes_l0 (t) :
    level_target_bytes (0#u32) t = ok (0#u64) := by
  unfold level_target_bytes
  rfl

/-- AS-IS dente: L0 target is still zero. -/
theorem level_target_bytes_as_is_l0 (t) :
    level_target_bytes_as_is (0#u32) t = ok (0#u64) := by
  unfold level_target_bytes_as_is
  rfl

/-- Any ok-valued Result bind forces the bound term to be ok. -/
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

/-- Catalog entry: the leveled size target of a level computes to a
    value exactly along one of two dispositions — level 0 has no
    target (the value is 0), or the level is non-zero and every step of
    the saturating chain level−1, min 18, FANOUT^·, l1_target·· is ok,
    the final saturating_mul producing the value (F-leveling-sweep). -/
theorem level_target_bytes_ok_iff_zero_or_fanout_chain :
    ∀ (level : U32) (l1_target : U64) (v : U64),
    (level_target_bytes level l1_target = ok v) ↔
      ((level = 0#u32 ∧ v = 0#u64) ∨
        (¬(level = 0#u32) ∧ ∃ i e f,
          level - 1#u32 = ok i ∧
          core.cmp.Ord.min.trait_default core.cmp.OrdU32 i 18#u32 = ok e ∧
          core.num.U64.saturating_pow LEVEL_FANOUT e = ok f ∧
          core.num.U64.saturating_mul l1_target f = ok v)) := by
  intro level l1_target v
  unfold level_target_bytes
  split
  · next h =>
    constructor
    · intro hval
      injection hval with hv
      exact Or.inl ⟨h, hv.symm⟩
    · rintro (⟨h0, hv⟩ | ⟨hne, i, e, f, hsub, hmin, hpow, hmul⟩)
      · rw [hv]
      · exact absurd h hne
  · next h =>
    constructor
    · intro hval
      obtain ⟨i, hsub, hval⟩ := bind_ok_inv _ _ _ hval
      obtain ⟨e, hmin, hval⟩ := bind_ok_inv _ _ _ hval
      obtain ⟨f, hpow, hval⟩ := bind_ok_inv _ _ _ hval
      exact Or.inr ⟨h, i, e, f, hsub, hmin, hpow, hval⟩
    · rintro (⟨h0, hv⟩ | ⟨hne, i, e, f, hsub, hmin, hpow, hmul⟩)
      · exact absurd h0 h
      · refine bind_intro i hsub ?_
        refine bind_intro e hmin ?_
        refine bind_intro f hpow ?_
        exact hmul
