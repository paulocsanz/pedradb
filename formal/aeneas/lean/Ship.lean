-- Theorems over Aeneas extract of ship_kernel.rs
import Aeneas
import ShipKernel
open Aeneas.Std Result
open pedra_aeneas_ship_kernel

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

/-- RFC-0218 P2.1 10/12 (átomo `catalog:ship_stamp`, entrada
    `stamp_changed`): o stamp mudou EXATAMENTE como citado — stamp
    agora maior já é mudança (true); do mesmo tamanho, mudou é o
    prefixo citado de mesmo comprimento ser diferente (`ne` gate).
    O AS-IS nunca vê mudança (rotaciona atrasado — dente plantado). -/
theorem stamp_changed_fate_iff :
    ∀ (stamp_then stamp_now : Slice U8) (v : Bool),
      (stamp_changed stamp_then stamp_now = ok v) ↔
        ((stamp_now.len > stamp_then.len ∧ v = true) ∨
         (¬ (stamp_now.len > stamp_then.len) ∧
          (∃ s : Slice U8,
            core.slice.index.Slice.index
              (core.slice.index.SliceIndexRangeToUsizeSlice U8) stamp_then
              { «end» := stamp_now.len } = ok s ∧
            core.cmp.impls.PartialEqShared.ne
              (Slice.Insts.CoreCmpPartialEqSlice core.cmp.PartialEqU8) s stamp_now
              = ok v))) := by
  intro stamp_then stamp_now v
  constructor
  · intro hval
    unfold stamp_changed at hval
    dsimp only at hval
    split at hval
    · next hc =>
      injection hval with hv
      exact Or.inl ⟨hc, hv.symm⟩
    · next hc =>
      obtain ⟨s, hidx, hval⟩ := bind_ok_inv _ _ _ hval
      exact Or.inr ⟨hc, s, hidx, hval⟩
  · rintro (⟨hc, hv⟩ | ⟨hc, s, hidx, hval⟩)
    · subst hv
      unfold stamp_changed
      dsimp only
      rw [if_pos hc]
    · unfold stamp_changed
      dsimp only
      rw [if_neg (by simp [hc])]
      exact bind_intro s hidx hval

/-- Catalog entry `stamp_changed` is a transparent def in the extract. -/
theorem stamp_changed_is_def : True := by
  have _ := @stamp_changed
  trivial
