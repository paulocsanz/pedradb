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

/-- RFC-0218 P2.1 11/12 (átomo `catalog:ship_guard`, entrada
    `pull_plan`): o plano de pull é EXATAMENTE a árvore citada —
    arquivo sumiu: Rotated (cursor atrasado ou stamp_then presente
    segura); arquivo de mesmo comprimento: UpToDate; arquivo
    maior: Ship do mínimo entre (len - cursor) e max_pull, com o
    gate citado stamp_changed forçando Rotated quando o stamp
    girou. O AS-IS nunca rotaciona no stamp (pull atrasado lê
    truncado — dente plantado). -/
theorem pull_plan_fate_iff :
    ∀ (file_len : Option U64) (cursor : U64) (max_pull : U64)
      (stamp_then : Option (Slice U8)) (stamp_now : Slice U8)
      (p : PullPlan),
      (pull_plan file_len cursor max_pull stamp_then stamp_now = ok p) ↔
        ((file_len = none ∧
          ((cursor > 0#u64 ∧ p = PullPlan.Rotated 0#u64 cursor) ∨
           (¬ (cursor > 0#u64) ∧
            ((core.option.Option.is_some stamp_then = true ∧
              p = PullPlan.Rotated 0#u64 cursor) ∨
             (core.option.Option.is_some stamp_then = false ∧
              p = PullPlan.UpToDate)))))
         ∨ (∃ len : U64, file_len = some len ∧
          ((len < cursor ∧ p = PullPlan.Rotated len cursor) ∨
           (¬ (len < cursor) ∧
            ((stamp_then = none ∧
              ((len = cursor ∧ p = PullPlan.UpToDate) ∨
               (¬ (len = cursor) ∧
                ∃ i i1 : U64, len - cursor = ok i ∧
                  core.cmp.Ord.min.trait_default core.cmp.OrdU64 i max_pull
                    = ok i1 ∧
                  p = PullPlan.Ship i1)))
             ∨ (∃ then1 : Slice U8, stamp_then = some then1 ∧
                ∃ b : Bool, stamp_changed then1 stamp_now = ok b ∧
                 ((b = true ∧ p = PullPlan.Rotated len cursor) ∨
                  (b = false ∧
                   ((len = cursor ∧ p = PullPlan.UpToDate) ∨
                    (¬ (len = cursor) ∧
                     ∃ i i1 : U64, len - cursor = ok i ∧
                       core.cmp.Ord.min.trait_default core.cmp.OrdU64 i
                         max_pull = ok i1 ∧
                       p = PullPlan.Ship i1)))))))))) := by
  intro file_len cursor max_pull stamp_then stamp_now p
  constructor
  · intro hval
    unfold pull_plan at hval
    cases file_len with
    | none =>
      dsimp only at hval
      refine Or.inl ⟨rfl, ?_⟩
      split at hval
      · next hc =>
        injection hval with hv
        exact Or.inl ⟨hc, hv.symm⟩
      · next hc =>
        refine Or.inr ⟨hc, ?_⟩
        split at hval
        · next hb =>
          injection hval with hv
          exact Or.inl ⟨hb, hv.symm⟩
        · next hb =>
          simp only [Bool.not_eq_true] at hb
          injection hval with hv
          exact Or.inr ⟨hb, hv.symm⟩
    | some len =>
      dsimp only at hval
      refine Or.inr ⟨len, rfl, ?_⟩
      split at hval
      · next hc =>
        injection hval with hv
        exact Or.inl ⟨hc, hv.symm⟩
      · next hc =>
        refine Or.inr ⟨hc, ?_⟩
        cases stamp_then with
        | none =>
          dsimp only at hval
          refine Or.inl ⟨rfl, ?_⟩
          split at hval
          · next he =>
            injection hval with hv
            exact Or.inl ⟨he, hv.symm⟩
          · next he =>
            obtain ⟨i, hi, hval⟩ := bind_ok_inv _ _ _ hval
            obtain ⟨i1, hm, hval⟩ := bind_ok_inv _ _ _ hval
            injection hval with hv
            exact Or.inr ⟨he, i, i1, hi, hm, hv.symm⟩
        | some then1 =>
          dsimp only at hval
          refine Or.inr ⟨then1, rfl, ?_⟩
          obtain ⟨b, hsc, hval⟩ := bind_ok_inv _ _ _ hval
          refine ⟨b, hsc, ?_⟩
          split at hval
          · next hb =>
            injection hval with hv
            exact Or.inl ⟨hb, hv.symm⟩
          · next hb =>
            simp only [Bool.not_eq_true] at hb
            refine Or.inr ⟨hb, ?_⟩
            split at hval
            · next he =>
              injection hval with hv
              exact Or.inl ⟨he, hv.symm⟩
            · next he =>
              obtain ⟨i, hi, hval⟩ := bind_ok_inv _ _ _ hval
              obtain ⟨i1, hm, hval⟩ := bind_ok_inv _ _ _ hval
              injection hval with hv
              exact Or.inr ⟨he, i, i1, hi, hm, hv.symm⟩
  · rintro ((⟨rfl, (⟨hc, hv⟩ | ⟨hc, (⟨hb, hv⟩ | ⟨hb, hv⟩)⟩)⟩) |
            ⟨len, rfl, (⟨hc, hv⟩ |
             ⟨hc, (⟨rfl, (⟨he, hv⟩ | ⟨he, i, i1, hi, hm, hv⟩)⟩ |
              ⟨then1, rfl, b, hsc, (⟨hb, hv⟩ |
               ⟨hb, (⟨he, hv⟩ | ⟨he, i, i1, hi, hm, hv⟩)⟩)⟩)⟩)⟩)
    · subst hv
      unfold pull_plan
      dsimp only
      rw [if_pos hc]
    · subst hv
      unfold pull_plan
      dsimp only
      rw [if_neg hc, if_pos hb]
    · subst hv
      unfold pull_plan
      dsimp only
      rw [if_neg hc, if_neg ((Bool.not_eq_true _).mpr hb)]
    · subst hv
      unfold pull_plan
      dsimp only
      rw [if_pos hc]
    · subst hv
      unfold pull_plan
      dsimp only
      rw [if_neg hc, if_pos he]
    · subst hv
      unfold pull_plan
      dsimp only
      rw [if_neg hc, if_neg he]
      exact bind_intro i hi (bind_intro i1 hm rfl)
    · subst hv
      unfold pull_plan
      dsimp only
      rw [if_neg hc]
      refine bind_intro b hsc ?_
      rw [if_pos hb]
    · subst hv
      unfold pull_plan
      dsimp only
      rw [if_neg hc]
      refine bind_intro b hsc ?_
      rw [if_neg ((Bool.not_eq_true _).mpr hb), if_pos he]
    · subst hv
      unfold pull_plan
      dsimp only
      rw [if_neg hc]
      refine bind_intro b hsc ?_
      rw [if_neg ((Bool.not_eq_true _).mpr hb), if_neg he]
      exact bind_intro i hi (bind_intro i1 hm rfl)
