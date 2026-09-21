-- Theorems over Aeneas extract of fold_kernel.rs
import Aeneas
import FoldKernel
open Aeneas.Std Result
open pedra_aeneas_fold_kernel

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

/-- RFC-0218 P2.1 9/12 (atom `catalog:fold_range`, entrada
    `fold_event_hides_key`): o fold esconde a chave EXATAMENTE como
    citado — evento de range esconde `key >= start` E `key < end`
    (gates citados em cascata); evento pontual esconde só a
    igualdade `key = start`. O AS-IS mostra tudo (segredo vaza no
    fold — tooth plantado). -/
theorem fold_event_hides_key_fate_iff :
    ∀ (is_range : Bool) (start : Slice U8) (end1 : Slice U8)
      (key : Slice U8) (v : Bool),
      (fold_event_hides_key is_range start end1 key = ok v) ↔
        ((is_range = true ∧
          (∃ b : Bool,
            Shared1A.Insts.CoreCmpPartialOrdShared0B.ge
              (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8)
              key start = ok b ∧
            ((b = true ∧
              Shared1A.Insts.CoreCmpPartialOrdShared0B.lt
                (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8)
                key end1 = ok v)
             ∨ (b = false ∧ v = false))))
         ∨ (is_range = false ∧
            core.slice.cmp.PartialEqSlice.eq core.cmp.PartialEqU8 key start
              = ok v)) := by
  intro is_range start end1 key v
  constructor
  · intro hval
    unfold fold_event_hides_key at hval
    split at hval
    · next hir =>
      obtain ⟨b, hgate, hval⟩ := bind_ok_inv _ _ _ hval
      refine Or.inl ⟨hir, b, hgate, ?_⟩
      split at hval
      · next hb => exact Or.inl ⟨hb, hval⟩
      · next hb =>
        simp only [Bool.not_eq_true] at hb
        injection hval with hv
        exact Or.inr ⟨hb, hv.symm⟩
    · next hir =>
      simp only [Bool.not_eq_true] at hir
      exact Or.inr ⟨hir, hval⟩
  · rintro ((⟨hir, b, hgate, (⟨hb, hlt⟩ | ⟨hb, hv⟩)⟩) | ⟨hir, hv⟩)
    · subst hir
      subst hb
      unfold fold_event_hides_key
      rw [if_pos rfl]
      exact bind_intro true hgate hlt
    · subst hir
      subst hb
      subst hv
      unfold fold_event_hides_key
      rw [if_pos rfl]
      exact bind_intro false hgate rfl
    · subst hir
      unfold fold_event_hides_key
      rw [if_neg (by simp)]
      exact hv
