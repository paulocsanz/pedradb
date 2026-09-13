-- Theorems over the Aeneas extract of production apply_kernel.rs
-- (RFC-0053 Y2.2 / RFC-0056 P1.2): second machine (not the Verus twin).
import Aeneas
import ApplyKernel
open Aeneas Std Result
open pedra_aeneas_apply_kernel

/-- Closed form: the apply loop only ever advances inside the contiguous
committed prefix (F10-apply). -/
theorem apply_advance_closed_form (la ci : Std.U64) (present : Bool) :
    apply_advance la ci present =
      (if la >= ci then ok ApplyAction.Done
       else if present then ok ApplyAction.Apply else ok ApplyAction.Stop) := by
  unfold apply_advance
  split
  · rfl
  · split <;> rfl

/-- At/below commit ⇒ Done (no runaway apply past commit_index). -/
theorem done_at_and_above_commit :
    apply_advance (5#u64) (5#u64) true = ok ApplyAction.Done ∧
      apply_advance (9#u64) (5#u64) false = ok ApplyAction.Done := by
  constructor <;> rfl

/-- A hole in the log ⇒ Stop — the store never applies outside the
contiguous committed prefix. -/
theorem missing_entry_stops :
    apply_advance (1#u64) (2#u64) false = ok ApplyAction.Stop := by
  rfl

/-- Present entry below commit ⇒ Apply. -/
theorem present_entry_applies :
    apply_advance (1#u64) (2#u64) true = ok ApplyAction.Apply := by
  rfl

/-- AS-IS teeth: the skip-holes mutant applies a missing entry — exactly
the hole the fixed kernel stops at. -/
theorem as_is_applies_hole :
    apply_advance_as_is_skip_holes (1#u64) (2#u64) false = ok ApplyAction.Apply ∧
      apply_advance (1#u64) (2#u64) false = ok ApplyAction.Stop := by
  constructor <;> rfl

/-- RFC-0218 P2.2 (átomo `catalog:apply_step`, entrada
    `apply_advance`): o passo de aplicação é exatamente a árvore citada
    — na frente do commit, Done; atrás do commit, Apply só com entrada
    presente, buraco é Stop (o prefixo contíguo é a fronteira). O
    AS-IS aplica o buraco (dente plantado). -/
theorem apply_advance_fate_iff :
    ∀ (last_applied commit_index : U64) (entry_present : Bool)
      (v : ApplyAction),
      (apply_advance last_applied commit_index entry_present = ok v) ↔
        ((v = ApplyAction.Done ∧ last_applied >= commit_index)
         ∨ (v = ApplyAction.Apply ∧ ¬(last_applied >= commit_index) ∧
              entry_present = true)
         ∨ (v = ApplyAction.Stop ∧ ¬(last_applied >= commit_index) ∧
              entry_present = false)) := by
  intro last_applied commit_index entry_present v
  constructor
  · intro hval
    unfold apply_advance at hval
    split at hval
    · next hc =>
      injection hval with hv
      exact Or.inl ⟨hv.symm, hc⟩
    · next hc =>
      split at hval
      · next hp =>
        injection hval with hv
        exact Or.inr (Or.inl ⟨hv.symm, hc, hp⟩)
      · next hp =>
        simp only [Bool.not_eq_true] at hp
        injection hval with hv
        exact Or.inr (Or.inr ⟨hv.symm, hc, hp⟩)
  · rintro (⟨rfl, hc⟩ | ⟨rfl, hc, hp⟩ | ⟨rfl, hc, hp⟩)
    · unfold apply_advance
      rw [if_pos hc]
    · unfold apply_advance
      rw [if_neg hc, if_pos hp]
    · unfold apply_advance
      have hp' : ¬(entry_present = true) := by simp [hp]
      rw [if_neg hc, if_neg hp']
