-- Theorems over Aeneas extract of properties_kernel.rs (RFC-0166 spec
-- crown). Loops use `loop.spec_decr_nat` + `step` on `index_usize_spec`
-- (mold: Isolated.lean). Lengths are always `x.val.length : Nat`.
-- Do not `simp [loop]`.
import Aeneas
import PropertiesKernel
open Aeneas.Std Result
open Aeneas.Std.WP
open pedra_aeneas_properties_kernel

/-- Any ok-valued Result bind forces the bound term to be ok
(Cf.lean's `bind_ok_inv`, restated for this module). -/
private theorem bind_ok_inv {α β} (x : Result α) (f : α → Result β) (v : β)
    (h : Aeneas.Std.bind x f = ok v) : ∃ a, x = ok a ∧ f a = ok v := by
  cases x with
  | ok a => exact ⟨a, rfl, h⟩
  | fail e => exact absurd h (by simp)
  | div => exact absurd h (by simp)

/-! ## RFC-0215 P0.1 — coroa de produto no degrau átomo (spec ×4) -/

/-- C1 semântica (forma-ramo: cita `majority` só onde o corpo chama):
valor servido passa quando ¬servido, ou a maioria antiga atingiu e
(single) vale, ou (joint) a nova também atingiu. -/
def c1_pass (old_n old_yes : U64) (joint : Bool) (new_n new_yes : U64)
    (served : Bool) : Prop :=
  served = false ∨
    (∃ m, majority old_n = ok m ∧ ¬ (old_yes < m) ∧
      (joint = false ∨
        (∃ m2, majority new_n = ok m2 ∧ ¬ (new_yes < m2))))

/-- C1 falha: servido sem maioria antiga, ou joint sem maioria nova. -/
def c1_fail (old_n old_yes : U64) (joint : Bool) (new_n new_yes : U64)
    (served : Bool) : Prop :=
  (served = true ∧ ∃ m, majority old_n = ok m ∧ (old_yes < m)) ∨
    (served = true ∧ joint = true ∧
      ∃ m m2, majority old_n = ok m ∧ ¬ (old_yes < m) ∧
        majority new_n = ok m2 ∧ (new_yes < m2))

/-- RFC-0215 P0.1 1/4 (atom `catalog:c1_quorum`, entry `c1_holds`):
um valor servido passa C1 exatamente quando a maioria de TODA config
ativa replica — joint exige antiga E nova. O mutante AS-IS
(`c1_holds_as_is`) aceita a maioria antiga sozinha (o buraco
joint-election, RFC-0064); planta `c1_as_is_does_not_imply_c1` recusa.
Fate forall sobre o corpo extraído (sem loop; `majority` citado,
corpo não reaberto). -/
theorem c1_holds_fate_iff :
    ∀ (old_n old_yes : U64) (joint : Bool) (new_n new_yes : U64)
      (served : Bool) (v : Bool),
      (c1_holds old_n old_yes joint new_n new_yes served = ok v) ↔
        ((v = true ∧ c1_pass old_n old_yes joint new_n new_yes served) ∨
          (v = false ∧ c1_fail old_n old_yes joint new_n new_yes served)) := by
  intro old_n old_yes joint new_n new_yes served v
  unfold c1_holds
  constructor
  · intro hval
    split at hval
    · -- served = true
      next hserved =>
        obtain ⟨m, hm, hval⟩ := bind_ok_inv _ _ _ hval
        split at hval
        · next hlt =>
            refine Or.inr ⟨(Result.ok.inj hval).symm,
              Or.inl ⟨hserved, m, hm, hlt⟩⟩
        · next hge =>
            split at hval
            · next hjoint =>
                obtain ⟨m2, hm2, hval⟩ := bind_ok_inv _ _ _ hval
                split at hval
                · next hlt2 =>
                    refine Or.inr ⟨(Result.ok.inj hval).symm,
                      Or.inr ⟨hserved, hjoint, m, m2, hm, hge, hm2, hlt2⟩⟩
                · next hge2 =>
                    refine Or.inl ⟨(Result.ok.inj hval).symm,
                      Or.inr ⟨m, hm, hge, Or.inr ⟨m2, hm2, hge2⟩⟩⟩
            · next hnjoint =>
                simp only [Bool.not_eq_true] at hnjoint
                refine Or.inl ⟨(Result.ok.inj hval).symm,
                  Or.inr ⟨m, hm, hge, Or.inl hnjoint⟩⟩
    · -- served = false
      next hnserved =>
        simp only [Bool.not_eq_true] at hnserved
        refine Or.inl ⟨(Result.ok.inj hval).symm, Or.inl hnserved⟩
  · intro hdisj
    cases hdisj with
    | inl hh =>
        obtain ⟨hv, hpass⟩ := hh
        subst hv
        rcases hpass with hs | ⟨m, hm, hge, hjoint⟩
        · -- served = false
          split
          · next hs'' => exact absurd hs'' (by rw [hs]; simp)
          · rfl
        · -- pass-∃: served pode ser true (prosseguir) ou false (corpo ok true)
          split
          · next _ =>
              simp only [hm, Aeneas.Std.bind_tc_ok]
              split
              · next hlt' => exact absurd hlt' hge
              · next _ =>
                  split
                  · next hj'' =>
                      rcases hjoint with hnjoint | ⟨m2, hm2, hge2⟩
                      · exact absurd hj'' (by rw [hnjoint]; simp)
                      · simp only [hm2, Aeneas.Std.bind_tc_ok]
                        split
                        · next hlt2' => exact absurd hlt2' hge2
                        · rfl
                  · rfl
          · rfl
    | inr hh =>
        obtain ⟨hv, hfail⟩ := hh
        subst hv
        rcases hfail with ⟨hserved, m, hm, hlt⟩ |
          ⟨hserved, hjoint, m, m2, hm, hge, hm2, hlt2⟩
        · rw [hserved, if_pos rfl]
          simp only [hm, Aeneas.Std.bind_tc_ok]
          rw [if_pos hlt]
        · rw [hserved, if_pos rfl]
          simp only [hm, Aeneas.Std.bind_tc_ok]
          rw [if_neg hge, if_pos hjoint]
          simp only [hm2, Aeneas.Std.bind_tc_ok]
          rw [if_pos hlt2]

