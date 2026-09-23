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

/-- T1 semântica: all-or-nothing — nunca ambos flags, índices visíveis
nomeiam writes staged, committed ⇒ todos visíveis, senão nada. -/
def t1_ok (committed aborted : Bool) (staged_n : Usize)
    (visible : Slice Usize) : Prop :=
  (committed = true → aborted = false) ∧
  (∀ j : Nat, (hj : j < visible.val.length) →
    (visible.val[j]).val < staged_n.val) ∧
  (committed = true → visible.val.length = staged_n.val) ∧
  (committed = false → visible.val.length = 0)

private theorem t1_loop0_spec (staged_n : Usize) (visible : Slice Usize)
    (j0 : Usize) (hInv : j0.val ≤ visible.val.length)
    (hpre : ∀ k : Nat, (hk : k < j0.val) →
      (hkl : k < visible.val.length) →
      (visible.val[k]).val < staged_n.val) :
    spec (t1_holds_loop0 staged_n visible j0)
      (fun b => (b = true) ↔
        ((∀ j : Nat, (hj : j < visible.val.length) →
            (visible.val[j]).val < staged_n.val) ∧
          visible.val.length = staged_n.val)) := by
  unfold t1_holds_loop0
  refine loop.spec_decr_nat
    (fun j => visible.val.length - j.val)
    (fun j => j.val ≤ visible.val.length ∧
      ∀ k : Nat, (hk : k < j.val) →
        (hkl : k < visible.val.length) →
        (visible.val[k]).val < staged_n.val)
    (fun b => (b = true) ↔
      ((∀ j : Nat, (hj : j < visible.val.length) →
          (visible.val[j]).val < staged_n.val) ∧
        visible.val.length = staged_n.val))
    (t1_holds_loop0.body staged_n visible) j0 ?body
    ⟨hInv, hpre⟩
  intro j ⟨hjle, hclean⟩
  unfold t1_holds_loop0.body
  dsimp +zeta only
  split
  · -- j < len
    rename_i hltU
    have hlt : j.val < visible.val.length := by
      simpa [UScalar.lt_equiv, Aeneas.Std.Slice.len_val] using hltU
    step as ⟨ x, hx ⟩
    have hxv : x = visible.val[j.val] := hx
    split
    · -- x >= staged_n : done false
      rename_i hgeU
      have hge : ¬ ((visible.val[j.val]).val < staged_n.val) := by
        rw [← hxv]
        have hN : (↑staged_n : Nat) ≤ (↑x : Nat) := by
          simpa [ge_iff_le, UScalar.le_equiv] using hgeU
        omega
      simp only [spec_ok, Bool.false_eq_true, false_iff]
      intro hall
      exact hge (hall.1 j.val hlt)
    · -- x < staged_n : cont
      rename_i hltU2
      have hlt2 : (visible.val[j.val]).val < staged_n.val := by
        rw [← hxv]
        simpa [UScalar.lt_equiv] using hltU2
      step as ⟨ j', hj' ⟩
      have hjv : (↑j' : Nat) = (↑j : Nat) + 1 := by simpa using hj'
      refine ⟨?le, ?clean, ?meas⟩
      · omega
      · intro k hk hkl
        rcases Nat.lt_or_ge k j.val with hkj | hkj
        · exact hclean k hkj hkl
        · have hk : k = j.val := by omega
          subst hk
          exact hlt2
      · omega
  · -- j >= len : done (Slice.len visible = staged_n)
    rename_i hgeU
    have hge : ¬ (j.val < visible.val.length) := by
      simpa [UScalar.lt_equiv, Aeneas.Std.Slice.len_val] using hgeU
    have hj_eq : j.val = visible.val.length :=
      Nat.le_antisymm hjle (Nat.le_of_not_lt hge)
    simp only [spec_ok]
    constructor
    · intro hbeq
      refine ⟨fun j' hj' => hclean j' (by omega) hj', ?_⟩
      have hEq : Slice.len visible = staged_n := by simpa using hbeq
      have hN := UScalar.eq_equiv _ _ |>.mp hEq
      rw [Aeneas.Std.Slice.len_val] at hN
      exact hN
    · intro ⟨_, hlen⟩
      have hEq : Slice.len visible = staged_n :=
        UScalar.eq_imp _ _ (by
          rw [Aeneas.Std.Slice.len_val]; exact hlen)
      exact decide_eq_true hEq

private theorem t1_loop1_spec (staged_n : Usize) (visible : Slice Usize)
    (j0 : Usize) (hInv : j0.val ≤ visible.val.length)
    (hpre : ∀ k : Nat, (hk : k < j0.val) →
      (hkl : k < visible.val.length) →
      (visible.val[k]).val < staged_n.val) :
    spec (t1_holds_loop1 staged_n visible j0)
      (fun b => (b = true) ↔
        ((∀ j : Nat, (hj : j < visible.val.length) →
            (visible.val[j]).val < staged_n.val) ∧
          visible.val.length = 0)) := by
  unfold t1_holds_loop1
  refine loop.spec_decr_nat
    (fun j => visible.val.length - j.val)
    (fun j => j.val ≤ visible.val.length ∧
      ∀ k : Nat, (hk : k < j.val) →
        (hkl : k < visible.val.length) →
        (visible.val[k]).val < staged_n.val)
    (fun b => (b = true) ↔
      ((∀ j : Nat, (hj : j < visible.val.length) →
          (visible.val[j]).val < staged_n.val) ∧
        visible.val.length = 0))
    (t1_holds_loop1.body staged_n visible) j0 ?body
    ⟨hInv, hpre⟩
  intro j ⟨hjle, hclean⟩
  unfold t1_holds_loop1.body
  dsimp +zeta only
  split
  · -- j < len
    rename_i hltU
    have hlt : j.val < visible.val.length := by
      simpa [UScalar.lt_equiv, Aeneas.Std.Slice.len_val] using hltU
    step as ⟨ x, hx ⟩
    have hxv : x = visible.val[j.val] := hx
    split
    · -- x >= staged_n : done false
      rename_i hgeU
      have hge : ¬ ((visible.val[j.val]).val < staged_n.val) := by
        rw [← hxv]
        have hN : (↑staged_n : Nat) ≤ (↑x : Nat) := by
          simpa [ge_iff_le, UScalar.le_equiv] using hgeU
        omega
      simp only [spec_ok, Bool.false_eq_true, false_iff]
      intro hall
      exact hge (hall.1 j.val hlt)
    · -- x < staged_n : cont
      rename_i hltU2
      have hlt2 : (visible.val[j.val]).val < staged_n.val := by
        rw [← hxv]
        simpa [UScalar.lt_equiv] using hltU2
      step as ⟨ j', hj' ⟩
      have hjv : (↑j' : Nat) = (↑j : Nat) + 1 := by simpa using hj'
      refine ⟨?le, ?clean, ?meas⟩
      · omega
      · intro k hk hkl
        rcases Nat.lt_or_ge k j.val with hkj | hkj
        · exact hclean k hkj hkl
        · have hk : k = j.val := by omega
          subst hk
          exact hlt2
      · omega
  · -- j >= len : done (is_empty visible)
    rename_i hgeU
    have hge : ¬ (j.val < visible.val.length) := by
      simpa [UScalar.lt_equiv, Aeneas.Std.Slice.len_val] using hgeU
    have hj_eq : j.val = visible.val.length :=
      Nat.le_antisymm hjle (Nat.le_of_not_lt hge)
    step as ⟨ beq, hbe ⟩
    have hbeT : (beq = true) ↔ visible.val.length = 0 := by
      rw [hbe]
    rw [hbeT]
    constructor
    · intro h0
      exact ⟨fun j' hj' => hclean j' (by omega) hj', h0⟩
    · intro ⟨_, hlen⟩
      exact hlen

/-- RFC-0215 P0.1 3/4 (atom `catalog:t1_atomicity`, entry `t1_holds`):
T1 aceita exatamente quando a tx é all-or-nothing — nunca ambos os
flags, visíveis nomeiam staged, committed ⇒ todos, senão nenhum. O
mutante AS-IS (`t1_holds_as_is`) só confere integridade de bytes (tx
abortada com efeito parcial passa); planta `t1_as_is_does_not_imply_t1`
recusa. Fate forall sobre o corpo extraído (dois loops reais via
`loop.spec_decr_nat`). -/
theorem t1_holds_fate_iff :
    ∀ (committed aborted : Bool) (staged_n : Usize)
      (visible : Slice Usize) (v : Bool),
      (t1_holds committed aborted staged_n visible = ok v) ↔
        ((v = true ∧ t1_ok committed aborted staged_n visible) ∨
          (v = false ∧ ¬ t1_ok committed aborted staged_n visible)) := by
  intro committed aborted staged_n visible v
  unfold t1_holds
  split
  · -- committed = true
    rename_i hcommitted
    have hcfalse : ¬ (committed = false) := by
      rw [hcommitted]; simp
    split
    · -- aborted = true : ok false
      next haborted =>
        constructor
        · intro hval
          rw [(Result.ok.inj hval).symm]
          exact Or.inr ⟨rfl,
            fun hp => absurd haborted (by simp [hp.1 hcommitted])⟩
        · intro hdisj
          rcases hdisj with ⟨hv, hp⟩ | ⟨hv, _⟩
          · exact absurd haborted (by simp [hp.1 hcommitted])
          · subst hv; rfl
    · -- aborted = false : loop0
      next haborted =>
        simp only [Bool.not_eq_true] at haborted
        obtain ⟨ b, hb, hpost ⟩ := (spec_equiv_exists _ _).mp
          (t1_loop0_spec staged_n visible 0#usize (Nat.zero_le _)
            (fun _ hk _ => absurd hk (Nat.not_lt.2 (Nat.zero_le _))))
        rw [hb]
        constructor
        · intro hval
          rw [(Result.ok.inj hval).symm]
          cases b with
          | true =>
              obtain ⟨hall, hlen⟩ := hpost.mp rfl
              exact Or.inl ⟨rfl, ⟨fun _ => haborted, hall,
                fun _ => hlen, fun hc => (hcfalse hc).elim⟩⟩
          | false =>
              refine Or.inr ⟨rfl, fun hp => ?_⟩
              obtain ⟨_, hall, hlen, _⟩ := hp
              exact absurd (hpost.mpr ⟨hall, hlen hcommitted⟩) (by simp)
        · intro hdisj
          cases hdisj with
          | inl hh =>
              obtain ⟨hv, _, hall, hlen, _⟩ := hh
              subst hv
              have hbt : b = true := by
                cases b with
                | true => rfl
                | false => exact absurd (hpost.mpr ⟨hall, hlen hcommitted⟩) (by simp)
              rw [hbt]
          | inr hh =>
              obtain ⟨hv, hneg⟩ := hh
              subst hv
              have hbf : b = false := by
                cases b with
                | true =>
                    exact absurd (hpost.mp rfl) (by
                      intro hp
                      obtain ⟨hall, hlen⟩ := hp
                      exact hneg ⟨fun _ => haborted, hall, fun _ => hlen,
                        fun hc => (hcfalse hc).elim⟩)
                | false => rfl
              rw [hbf]
  · -- committed = false : loop1
    next hctrue =>
      have hcf : committed = false := by
        simpa [Bool.not_eq_true] using hctrue
      obtain ⟨ b, hb, hpost ⟩ := (spec_equiv_exists _ _).mp
        (t1_loop1_spec staged_n visible 0#usize (Nat.zero_le _)
          (fun _ hk _ => absurd hk (Nat.not_lt.2 (Nat.zero_le _))))
      rw [hb]
      constructor
      · intro hval
        rw [(Result.ok.inj hval).symm]
        cases b with
        | true =>
            obtain ⟨hall, hlen0⟩ := hpost.mp rfl
            exact Or.inl ⟨rfl, ⟨fun hc => absurd hc hctrue, hall,
              fun hc => absurd hc hctrue, fun _ => hlen0⟩⟩
        | false =>
            refine Or.inr ⟨rfl, fun hp => ?_⟩
            obtain ⟨_, hall, _, hnone⟩ := hp
            exact absurd (hpost.mpr ⟨hall, hnone hcf⟩) (by simp)
      · intro hdisj
        cases hdisj with
        | inl hh =>
            obtain ⟨hv, _, hall, _, hnone⟩ := hh
            subst hv
            have hbt : b = true := by
              cases b with
              | true => rfl
              | false => exact absurd (hpost.mpr ⟨hall, hnone hcf⟩) (by simp)
            rw [hbt]
        | inr hh =>
            obtain ⟨hv, hneg⟩ := hh
            subst hv
            have hbf : b = false := by
              cases b with
              | true =>
                  exact absurd (hpost.mp rfl) (by
                    intro hp
                    obtain ⟨hall, hnone⟩ := hp
                    exact hneg ⟨fun hc => absurd hc hctrue, hall,
                      fun hc => absurd hc hctrue, fun _ => hnone⟩)
              | false => rfl
            rw [hbf]

/-- R1 semântica do primeiro hit a partir de `i0`: testemunha `k`
com `l[k] = some s`, todo anterior (≥ i0, < k) `none`; ou tudo `none`
e resposta `none`. Bounds viajam como ∃-provas (indexação plain
elabora com eles no contexto). -/
def IsFirstHitFrom (i0 : Nat) (l : List (Option Slot)) (o : Option Slot) :
    Prop :=
  (∃ (k : Nat) (s : Slot) (hk0 : i0 ≤ k) (hk : k < l.length),
      l[k] = some s ∧ o = some s ∧
        (∀ k' (hk0' : i0 ≤ k') (hk' : k' < k), l[k'] = none)) ∨
  (o = none ∧ ∀ k' (hk0' : i0 ≤ k') (hkl' : k' < l.length),
      l[k'] = none)

/-- R1 no vetor inteiro (i0 = 0, índice 0 = fonte mais nova). -/
def IsFirstHit (l : List (Option Slot)) (o : Option Slot) : Prop :=
  IsFirstHitFrom 0 l o

private theorem isFirstHitFrom_unique {i0 : Nat} {l : List (Option Slot)}
    {o o' : Option Slot} (h : IsFirstHitFrom i0 l o)
    (h' : IsFirstHitFrom i0 l o') : o = o' := by
  unfold IsFirstHitFrom at h h'
  rcases h with ⟨k, s, hk0, hk, hval, hoe, hpre⟩ | ⟨hone, hall⟩
  · rcases h' with ⟨k', s', hk0', hk', hval', hoe', hpre'⟩ | ⟨hone', hall'⟩
    · rcases Nat.lt_trichotomy k k' with hlt | heq | hgt
      · rw [hpre' k hk0 hlt] at hval
        simp at hval
      · subst heq
        rw [hoe, hoe', ← hval, hval']
      · rw [hpre k' hk0' hgt] at hval'
        simp at hval'
    · rw [hall' k hk0 hk] at hval
      simp at hval
  · rcases h' with ⟨k', s', hk0', hk', hval', hoe', hpre'⟩ | ⟨hone', hall'⟩
    · rw [hall k' hk0' hk'] at hval'
      simp at hval'
    · exact hone.trans hone'.symm

private theorem r1_loop_spec (probes : Slice (Option Slot)) (i0 : Usize)
    (hInv : i0.val ≤ probes.val.length) :
    spec (r1_first_hit_loop probes i0)
      (fun o => IsFirstHitFrom i0.val probes.val o) := by
  unfold r1_first_hit_loop
  refine loop.spec_decr_nat
    (fun j => probes.val.length - j.val)
    (fun j => i0.val ≤ j.val ∧ j.val ≤ probes.val.length ∧
      ∀ k : Nat, i0.val ≤ k → (hk : k < j.val) →
        (hkl : k < probes.val.length) → probes.val[k] = none)
    (fun o => IsFirstHitFrom i0.val probes.val o)
    (r1_first_hit_loop.body probes) i0 ?body
    ⟨Nat.le_refl _, hInv,
      fun _ hk0 hk _ => absurd hk (Nat.not_lt.mpr hk0)⟩
  intro j ⟨hj0, hjle, hnone⟩
  unfold r1_first_hit_loop.body
  dsimp +zeta only
  split
  · -- j < len
    rename_i hltU
    have hlt : j.val < probes.val.length := by
      simpa [UScalar.lt_equiv, Aeneas.Std.Slice.len_val] using hltU
    step as ⟨ o, ho ⟩
    split
    · -- is_some : done o
      rename_i hsome
      rcases o with _ | s
      · simp at hsome
      · refine Or.inl ⟨j.val, s, hj0, hlt, ho.symm, rfl,
          fun k' hk0' hk' => hnone k' hk0' hk' (by omega)⟩
    · -- none : cont
      rename_i hnos
      rcases o with _ | s
      · have hon : probes.val[j.val] = none := by rw [← ho]
        step as ⟨ j', hj' ⟩
        have hjv : (↑j' : Nat) = (↑j : Nat) + 1 := by simpa using hj'
        refine ⟨?le, ?le2, ?clean, ?meas⟩
        · omega
        · omega
        · intro k' hk0' hk' hkl
          rcases Nat.lt_or_ge k' j.val with hkj | hkj
          · exact hnone k' hk0' hkj hkl
          · have hk : k' = j.val := by omega
            subst hk
            exact hon
        · omega
      · simp at hnos
  · -- j >= len : done none
    rename_i hgeU
    have hge : ¬ (j.val < probes.val.length) := by
      simpa [UScalar.lt_equiv, Aeneas.Std.Slice.len_val] using hgeU
    have hj_eq : j.val = probes.val.length :=
      Nat.le_antisymm hjle (Nat.le_of_not_lt hge)
    refine Or.inr ⟨rfl, ?_⟩
    intro k' hk0' hkl'
    exact hnone k' hk0' (by omega) hkl'

/-- RFC-0215 P0.1 4/4, perna semântica: o primeiro hit é exatamente o
primeiro `some` na ordem de probe a partir do início (índice 0 =
fonte mais nova), com todos os anteriores `none`. -/
private theorem r1_first_hit_fate :
    ∀ (probes : Slice (Option Slot)) (o : Option Slot),
      (r1_first_hit probes = ok o) ↔ IsFirstHit probes.val o := by
  intro probes o
  obtain ⟨ b, hb, hpost ⟩ := (spec_equiv_exists _ _).mp
    (r1_loop_spec probes 0#usize (Nat.zero_le _))
  unfold r1_first_hit
  rw [hb]
  constructor
  · intro h
    rw [(Result.ok.inj h).symm]
    exact hpost
  · intro h
    have hob : o = b := isFirstHitFrom_unique h hpost
    rw [hob]

/-- RFC-0215 P0.1 4/4 (atom `catalog:r1_no_resumption`, entry
`r1_answer_ok`): a resposta de leitura passa R1 exatamente quando
bate com o primeiro hit (`r1_first_hit_fate` caracteriza o hit como
`IsFirstHit`; a igualdade de `Option` é axioma de extrato, citado não
reaberto). O mutante AS-IS (`r1_answer_ok_as_is`) aceita qualquer hit
cobridor (a ressurreição do delete de
findings/2026-09-04-reopen-delete-resurrected); planta
`r1_as_is_does_not_imply_r1` recusa. -/
theorem r1_answer_ok_fate_iff :
    ∀ (probes : Slice (Option Slot)) (answer : Option Slot) (v : Bool),
      (r1_answer_ok probes answer = ok v) ↔
        ∃ o, r1_first_hit probes = ok o ∧
          core.option.Option.Insts.CoreCmpPartialEqOption.eq
            Slot.Insts.CoreCmpPartialEqSlot answer o = ok v := by
  intro probes answer v
  constructor
  · intro hval
    unfold r1_answer_ok at hval
    obtain ⟨ o, ho, hval ⟩ := bind_ok_inv _ _ _ hval
    exact ⟨ o, ho, hval ⟩
  · rintro ⟨ o, ho, hval ⟩
    unfold r1_answer_ok
    rw [ho]
    exact hval
