-- Theorems over Aeneas extract of path_kernel.rs (origin-form routing).
-- Charon --exclude str Pattern methods; catalog-fn holes patched in
-- aeneas_path.sh.
import Aeneas
import PathKernel
open Aeneas.Std Result
open pedra_aeneas_path_kernel

/-- Catalog entry: authority-form targets are stripped. -/
theorem strip_authority_for_routing_true :
    strip_authority_for_routing true = ok true := by
  unfold strip_authority_for_routing
  rfl

/-- AS-IS dente: never strip authority. -/
theorem strip_authority_for_routing_as_is_dente :
    strip_authority_for_routing_as_is true = ok false := by
  unfold strip_authority_for_routing_as_is
  rfl

/-- AS-IS dente: fragment stays in the path. -/
theorem strip_uri_fragment_as_is_id (t) :
    strip_uri_fragment_as_is t = ok t := by
  unfold strip_uri_fragment_as_is
  rfl

/-- AS-IS dente: Host is never compared. -/
theorem host_authority_mismatch_as_is_dente (h a) :
    host_authority_mismatch_as_is h a = ok false := by
  unfold host_authority_mismatch_as_is
  rfl

/-! ### RFC-0216 P2.1 — path ×8 átomo -/

private theorem bind_ok_inv {α β} (x : Result α) (f : α → Result β) (v : β)
    (h : Aeneas.Std.bind x f = ok v) : ∃ a, x = ok a ∧ f a = ok v := by
  cases x with
  | ok a => exact ⟨a, rfl, h⟩
  | fail e => exact absurd h (by simp)
  | div => exact absurd h (by simp)

/-- RFC-0216 P2.1 1/8 (átomo `catalog:strip_authority_for_routing`):
  o roteador da forma-authority repassa exatamente a bandeira de
  forma-authority; o AS-IS nunca strips (dente já provado acima). -/
theorem strip_authority_for_routing_fate_iff :
    ∀ (b : Bool) (r : Bool),
      (strip_authority_for_routing b = ok r) ↔ r = b := by
  intro b r
  constructor
  · intro hval
    unfold strip_authority_for_routing at hval
    exact (Result.ok.inj hval).symm
  · intro hr
    unfold strip_authority_for_routing
    rw [hr]

/-- RFC-0216 P2.1 2/8 (átomo `catalog:strip_uri_fragment`): o
  fragmento é descartado exatamente pelo split no `#` — sem `#` a
  target volta inteira, com `#` fica o prefixo. -/
theorem strip_uri_fragment_fate_iff :
    ∀ (t : Str) (r : Str),
      (strip_uri_fragment t = ok r) ↔
        ((core.str.Str.split_once t '#' = ok none ∧ r = t) ∨
          (∃ (a : Str) (snd : Str),
              core.str.Str.split_once t '#' = ok (some (a, snd)) ∧ r = a)) := by
  intro t r
  constructor
  · intro hval
    unfold strip_uri_fragment at hval
    obtain ⟨o, ho, hval⟩ := bind_ok_inv _ _ _ hval
    cases o with
    | none =>
      dsimp only at hval
      exact Or.inl ⟨ho, (Result.ok.inj hval).symm⟩
    | some pair =>
      obtain ⟨a, snd⟩ := pair
      dsimp only at hval
      exact Or.inr ⟨a, snd, ho, (Result.ok.inj hval).symm⟩
  · rintro (⟨ho, rfl⟩ | ⟨a, snd, ho, rfl⟩)
    · unfold strip_uri_fragment
      rw [ho]
      simp only [Aeneas.Std.bind_tc_ok]
    · unfold strip_uri_fragment
      rw [ho]
      simp only [Aeneas.Std.bind_tc_ok]

/-- RFC-0216 P2.1 3/8 (átomo `catalog:path_after_authority`): o path
  depois da autoridade é exatamente o primeiro `/` em diante — sem
  `/` a resposta é a raiz `/`, com `/` é o slice index a partir
  dele. -/
theorem path_after_authority_fate_iff :
    ∀ (rest : Str) (r : Str),
      (path_after_authority rest = ok r) ↔
        ((core.str.Str.find rest '/' = ok none ∧
            r = toStr "/" path_after_authority._proof_1) ∨
          (∃ (i : Usize),
              core.str.Str.find rest '/' = ok (some i) ∧
                Str.Insts.CoreOpsIndexIndex.index
                  core.ops.range.RangeFromUsize.Insts.CoreSliceIndexSliceIndexStrStr
                  rest { start := i } = ok r)) := by
  intro rest r
  constructor
  · intro hval
    unfold path_after_authority at hval
    obtain ⟨o, ho, hval⟩ := bind_ok_inv _ _ _ hval
    cases o with
    | none =>
      dsimp only at hval
      exact Or.inl ⟨ho, (Result.ok.inj hval).symm⟩
    | some i =>
      dsimp only at hval
      exact Or.inr ⟨i, ho, hval⟩
  · rintro (⟨ho, rfl⟩ | ⟨i, ho, hindex⟩)
    · unfold path_after_authority
      rw [ho]
      simp only [Aeneas.Std.bind_tc_ok]
    · unfold path_after_authority
      rw [ho]
      simp only [Aeneas.Std.bind_tc_ok]
      exact hindex
