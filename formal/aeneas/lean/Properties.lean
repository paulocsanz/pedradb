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

/-- D1 semântica: nenhum acked perde o corte — todo índice ackado está
dentro do prefixo que sobrevive. -/
def d1_ok (acked : Slice Bool) (survives : Usize) : Prop :=
  ∀ i : Nat, (hi : i < acked.val.length) →
    acked.val[i] = true → i < survives.val

private theorem d1_loop_spec (acked : Slice Bool) (survives : Usize)
    (i0 : Usize) (hInv : i0.val ≤ acked.val.length) :
    spec (d1_holds_loop acked survives i0)
      (fun b => (b = true) ↔
        ∀ k : Nat, i0.val ≤ k → (hk : k < acked.val.length) →
          acked.val[k] = true → k < survives.val) := by
  unfold d1_holds_loop
  refine loop.spec_decr_nat
    (fun j => acked.val.length - j.val)
    (fun j => i0.val ≤ j.val ∧ j.val ≤ acked.val.length ∧
      ∀ k : Nat, i0.val ≤ k → (hk : k < j.val) →
        (hkl : k < acked.val.length) →
        acked.val[k] = true → k < survives.val)
    (fun b => (b = true) ↔
      ∀ k : Nat, i0.val ≤ k → (hk : k < acked.val.length) →
        acked.val[k] = true → k < survives.val)
    (d1_holds_loop.body acked survives) i0 ?body
    ⟨Nat.le_refl _, hInv,
      fun _ hk0 hk _ _ => absurd hk (Nat.not_lt.mpr hk0)⟩
  intro j ⟨hj0, hjle, hclean⟩
  unfold d1_holds_loop.body
  dsimp +zeta only
  split
  · -- j < len
    rename_i hltU
    have hlt : j.val < acked.val.length := by
      simpa [UScalar.lt_equiv, Aeneas.Std.Slice.len_val] using hltU
    step as ⟨ b, hb ⟩
    have hbv : b = acked.val[j.val] := hb
    split
    · -- acked[j] = true
      rename_i hbt
      split
      · -- j >= survives : done false, j é testemunha
        rename_i hgeU
        have hge : ¬ (j.val < survives.val) := by
          have hN : (↑survives : Nat) ≤ (↑j : Nat) := by
            simpa [ge_iff_le, UScalar.le_equiv] using hgeU
          omega
        simp only [spec_ok, Bool.false_eq_true, false_iff]
        intro hall
        exact hge (hall j.val hj0 hlt (hbv.symm.trans hbt))
      · -- j < survives : cont
        rename_i hlt2U
        have hlt2 : j.val < survives.val := by
          have h1 : ¬ (survives ≤ j) := by simpa [ge_iff_le] using hlt2U
          have h2 : ¬ ((↑survives : Nat) ≤ (↑j : Nat)) := fun hle =>
            h1 ((UScalar.le_equiv _ _).mpr hle)
          omega
        step as ⟨ j', hj' ⟩
        have hjv : (↑j' : Nat) = (↑j : Nat) + 1 := by simpa using hj'
        refine ⟨?le, ?le2, ?clean, ?meas⟩
        · omega
        · omega
        · intro k hk0 hk hkl htrue
          rcases Nat.lt_or_ge k j.val with hkj | hkj
          · exact hclean k hk0 hkj hkl htrue
          · have hk : k = j.val := by omega
            rw [hk]
            exact hlt2
        · omega
    · -- acked[j] = false : cont
      rename_i hbf
      step as ⟨ j', hj' ⟩
      have hjv : (↑j' : Nat) = (↑j : Nat) + 1 := by simpa using hj'
      refine ⟨?leF, ?le2F, ?cleanF, ?measF⟩
      · omega
      · omega
      · intro k hk0 hk hkl htrue
        rcases Nat.lt_or_ge k j.val with hkj | hkj
        · exact hclean k hk0 hkj hkl htrue
        · have hk : k = j.val := by omega
          subst hk
          rw [← hbv] at htrue
          rw [htrue] at hbf
          exact absurd rfl hbf
      · omega
  · -- j >= len : done true
    rename_i hgeU
    have hge : ¬ (j.val < acked.val.length) := by
      simpa [UScalar.lt_equiv, Aeneas.Std.Slice.len_val] using hgeU
    have hj_eq : j.val = acked.val.length :=
      Nat.le_antisymm hjle (Nat.le_of_not_lt hge)
    simp only [spec_ok]
    constructor
    · intro _
      intro k hk0 hk htrue
      exact hclean k hk0 (by omega) hk htrue
    · intro _
      exact trivial

/-- RFC-0215 P0.1 2/4 (atom `catalog:d1_durability`, entry `d1_holds`):
D1 aceita (acked, survives) exatamente quando nenhum índice ackado
cai em ou além do prefixo sobrevivente — put→Ok é durável, a promessa
G1. O mutante AS-IS (`d1_holds_as_is`) só promete barreira para os
synced (a classe sync=false do peer); planta `d1_as_is_does_not_imply_d1`
recusa. Fate forall sobre o corpo extraído (loop real via
`loop.spec_decr_nat`, semântica first-order sobre `Slice.val`). -/
theorem d1_holds_fate_iff :
    ∀ (acked : Slice Bool) (survives : Usize) (v : Bool),
      (d1_holds acked survives = ok v) ↔
        ((v = true ∧ d1_ok acked survives) ∨
          (v = false ∧ ¬ d1_ok acked survives)) := by
  intro acked survives v
  obtain ⟨ b, hb, hpost ⟩ := (spec_equiv_exists _ _).mp
    (d1_loop_spec acked survives 0#usize (Nat.zero_le _))
  have hP : d1_ok acked survives ↔
      ∀ k : Nat, (0#usize).val ≤ k → (hk : k < acked.val.length) →
        acked.val[k] = true → k < survives.val :=
    ⟨fun h k _ hk ht => h k hk ht, fun h k hi ht => h k (Nat.zero_le _) hi ht⟩
  unfold d1_holds
  rw [hb]
  constructor
  · intro hval
    rw [(Result.ok.inj hval).symm]
    cases b with
    | true => exact Or.inl ⟨rfl, hP.mpr (hpost.mp rfl)⟩
    | false =>
        refine Or.inr ⟨rfl, fun hp => ?_⟩
        exact absurd (hpost.mpr (hP.mp hp)) (by simp)
  · intro hdisj
    cases hdisj with
    | inl hh =>
        obtain ⟨hv, hp⟩ := hh
        subst hv
        have hbt : b = true := by
          cases b with
          | true => rfl
          | false => exact absurd (hpost.mpr (hP.mp hp)) (by simp)
        rw [hbt]
    | inr hh =>
        obtain ⟨hv, hp⟩ := hh
        subst hv
        have hbf : b = false := by
          cases b with
          | true => exact absurd (hP.mpr (hpost.mp rfl)) hp
          | false => rfl
        rw [hbf]

