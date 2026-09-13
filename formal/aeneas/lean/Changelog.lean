-- Theorems over Aeneas extract of changelog_kernel.rs
import Aeneas
import ChangelogKernel
open Aeneas.Std Result
open pedra_aeneas_changelog_kernel

theorem changelog_should_store_due :
    changelog_should_store 5#u64 3#u64 = ok true := by
  unfold changelog_should_store
  have hgt : (3#u64 > 0#u64) = true := by native_decide
  have hge : (5#u64 ≥ 3#u64) = true := by native_decide
  simp [hgt, hge]
/-- RFC-0218 P0.3 1/6 (átomo `catalog:changelog`): a decisão de
    rebuild é EXATAMENTE a janela citada — feed vazio com seq > 0
    precisa de rebuild; feed vivo nunca (o feed é a verdade). O AS-IS
    devolve sempre false (rebuild cego — dente plantado no modelo
    Stateright do fn real). -/
theorem changelog_needs_sst_rebuild_fate_iff :
    ∀ (feed_empty : Bool) (last_sequence : U64) (v : Bool),
      (changelog_needs_sst_rebuild feed_empty last_sequence = ok v) ↔
        ((feed_empty = true ∧ v = (decide (last_sequence > 0#u64) : Bool)) ∨
          (feed_empty = false ∧ v = false)) := by
  intro feed_empty last_sequence v
  cases feed_empty with
  | true =>
    constructor
    · intro hval
      simp only [changelog_needs_sst_rebuild] at hval
      injection hval with hv
      exact Or.inl ⟨rfl, hv.symm⟩
    · rintro (⟨-, hv⟩ | h2)
      · subst hv
        rfl
      · exact absurd h2.1 (fun h => Bool.noConfusion h)
  | false =>
    constructor
    · intro hval
      simp only [changelog_needs_sst_rebuild] at hval
      injection hval with hv
      exact Or.inr ⟨rfl, hv.symm⟩
    · rintro (h1 | ⟨-, hv⟩)
      · exact absurd h1.1 (fun h => Bool.noConfusion h)
      · subst hv
        rfl
