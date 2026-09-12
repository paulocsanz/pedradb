-- Theorems over Aeneas extract of cqe_kernel.rs (U1 / F203).
-- RUSTFLAGS=--cfg test so as_is mutants are visible; Atomic telemetry in
-- submit_complete_act stripped in aeneas_cqe.sh.
import Aeneas
import CqeKernel
open Aeneas.Std Result
open Aeneas.Std.WP
open pedra_aeneas_cqe_kernel

theorem cqe_res_ok_nonneg :
    cqe_res_ok 0#i32 = ok true := by
  unfold cqe_res_ok
  simp

/-- Harvested CQE is used even if submit returned Err. -/
theorem submit_complete_act_harvested :
    submit_complete_act false true = ok SubmitCompleteAct.UseHarvested := by
  unfold submit_complete_act
  rfl

/-- AS-IS dente: failed submit returns Err (releases the buffer). -/
theorem submit_complete_act_as_is_dente :
    submit_complete_act_as_is false false =
      ok SubmitCompleteAct.ReturnSubmitErr := by
  unfold submit_complete_act_as_is
  rfl

/-- AS-IS dente: constant per-opcode tag. -/
theorem next_user_data_as_is_dente :
    next_user_data_as_is (1#u64) (0x77#u64) = ok (0x77#u64, 1#u64) := by
  unfold next_user_data_as_is
  rfl

/-! ## RFC-0214 P1.1 — costura CQE no degrau átomo (fate ∀) -/

/-- RFC-0214 P1.1 (atom `catalog:cqe_res`): um CQE é sucesso
se, e somente se, `res >= 0` — res negativo é erro, o kernel não
inventa sucesso. Fate forall sobre o corpo extraído
(`cqe_res_ok`, res-gate do fsync no ring). O mutante AS-IS
(`cqe_res_ok_as_is`) promove todo CQE a sucesso — planta DST
`cqe_res_ok_on_live_uring_is_not_ok` recusa. -/
theorem cqe_res_ok_fate_iff :
    ∀ (res : I32) (v : Bool),
      (cqe_res_ok res = ok v) ↔ (v = ((res >= 0#i32) : Bool)) := by
  intro res v
  unfold cqe_res_ok
  constructor
  · intro h
    exact (Result.ok.inj h).symm
  · intro h
    rw [h]

/-- RFC-0214 P1.1 (atom `catalog:cqe_tags`): a próxima tag por
operação — de um contador `c ≠ 0` devolve `(c, c+1)` (wrapping);
de `c = 0` pula o zero reservado e devolve `(1, 2)`. Tags únicas
por SQE. Fate forall sobre o corpo extraído (`next_user_data`,
loop real do kernel via `loop.spec_decr_nat`). O mutante AS-IS
(`next_user_data_as_is`) devolve tag constante por opcode —
plantas DST `unique_tags_discard_leftover_same_opcode` e
`cqe_act_as_is_adopts_leftover` recusam. -/
theorem next_user_data_fate_iff :
    ∀ (c a b : U64),
      (next_user_data c = ok (a, b)) ↔
        ((c ≠ 0#u64 ∧ a = c ∧
            b = core.num.U64.wrapping_add c 1#u64)
          ∨ (c = 0#u64 ∧ a = 1#u64 ∧ b = 2#u64)) := by
  intro c a b
  have hlift : ∀ k : U64,
      lift (core.num.U64.wrapping_add k 1#u64)
        = ok (core.num.U64.wrapping_add k 1#u64) := fun _ => rfl
  have hloop_ne : ∀ k : U64, (k != 0#u64) = true →
      next_user_data_loop k
        = ok (k, core.num.U64.wrapping_add k 1#u64) := by
    intro k hk
    unfold next_user_data_loop
    have hwp : spec
        (loop next_user_data_loop.body k)
        (fun y => y = (k, core.num.U64.wrapping_add k 1#u64)) := by
      refine loop.spec_decr_nat
        (fun _ => 0)
        (fun x => x = k ∧ (k != 0#u64) = true)
        (fun y => y = (k, core.num.U64.wrapping_add k 1#u64))
        (next_user_data_loop.body) k ?body ⟨rfl, hk⟩
      intro x hx
      obtain ⟨hxk, hkne⟩ := hx
      subst hxk
      unfold next_user_data_loop.body
      rw [hlift x, bind_tc_ok]
      split
      · simp [spec_ok]
      · next hn =>
          exact absurd hkne hn
    obtain ⟨y, hy, hyy⟩ := (spec_equiv_exists _ _).mp hwp
    rw [hy, hyy]
  have hloop_zero : next_user_data_loop 0#u64 = ok (1#u64, 2#u64) := by
    unfold next_user_data_loop
    have hwp : spec
        (loop next_user_data_loop.body 0#u64)
        (fun y => y = (1#u64, 2#u64)) := by
      refine loop.spec_decr_nat
        (fun x => if x = 0#u64 then 2 else 1)
        (fun x => x = 0#u64 ∨ x = 1#u64)
        (fun y => y = (1#u64, 2#u64))
        (next_user_data_loop.body) 0#u64 ?body0 (Or.inl rfl)
      intro j hj
      unfold next_user_data_loop.body
      rw [hlift j, bind_tc_ok]
      split
      · -- j != 0: done — só pode ser j = 1
        next hne =>
          simp only [spec_ok]
          cases hj with
          | inl h0 =>
              rw [h0] at hne
              simp at hne
          | inr h1 =>
              rw [h1]
              have hw : core.num.U64.wrapping_add 1#u64 1#u64 = 2#u64 := by
                native_decide
              rw [hw]
      · -- j = 0: cont 1
        next hn =>
          cases hj with
          | inl h0 =>
              rw [h0]
              have hw : core.num.U64.wrapping_add 0#u64 1#u64 = 1#u64 := by
                native_decide
              rw [hw]
              simp [spec_ok]
          | inr h1 =>
              rw [h1] at hn
              simp at hn
    obtain ⟨y, hy, hyy⟩ := (spec_equiv_exists _ _).mp hwp
    rw [hy, hyy]
  unfold next_user_data
  cases hc : (c != 0#u64) with
  | true =>
      rw [hloop_ne c hc]
      constructor
      · intro h
        have hp := Result.ok.inj h
        simp only [Prod.mk.injEq] at hp
        have hne : c ≠ 0#u64 := by simpa using hc
        exact Or.inl ⟨hne, hp.1.symm, hp.2.symm⟩
      · intro hdisj
        cases hdisj with
        | inl hh =>
            rw [hh.2.1, hh.2.2]
        | inr hh =>
            rw [hh.1] at hc
            simp at hc
  | false =>
      have hc0 : c = 0#u64 := by simpa using hc
      rw [hc0, hloop_zero]
      constructor
      · intro h
        have hp := Result.ok.inj h
        simp only [Prod.mk.injEq] at hp
        exact Or.inr ⟨rfl, hp.1.symm, hp.2.symm⟩
      · intro hdisj
        cases hdisj with
        | inl hh =>
            exact absurd rfl hh.1
        | inr hh =>
            rw [hh.2.1, hh.2.2]

/-- RFC-0214 P1.1 (atom `catalog:cqe_leftover`): um CQE é
tomado se, e somente se, sua tag casa com a esperada; CQE
leftover de outra operação é descartado. Fate forall sobre o
corpo extraído (`cqe_act`). O mutante AS-IS
(`cqe_act_as_is`) toma qualquer CQE — a planta DST
`cqe_act_as_is_adopts_leftover` recusa. -/
theorem cqe_act_fate_iff :
    ∀ (user_data want : U64) (act : CqeAct),
      (cqe_act user_data want = ok act) ↔
        ((user_data = want ∧ act = CqeAct.Take)
          ∨ (user_data ≠ want ∧ act = CqeAct.Discard)) := by
  intro user_data want act
  unfold cqe_act
  split
  · next h =>
      constructor
      · intro he
        exact Or.inl ⟨h, (Result.ok.inj he).symm⟩
      · intro hdisj
        cases hdisj with
        | inl hh =>
            rw [hh.2]
        | inr hh =>
            exact absurd h hh.1
  · next hn =>
      constructor
      · intro he
        exact Or.inr ⟨hn, (Result.ok.inj he).symm⟩
      · intro hdisj
        cases hdisj with
        | inl hh =>
            exact absurd hh.1 hn
        | inr hh =>
            rw [hh.2]

/-- RFC-0214 P1.1 (atom `catalog:cqe_submit`): depois de um
submit, a decisão usa o CQE colhido se houver (`UseHarvested`)
e senão espera (`WaitMore`) — o SQE já está no anel, voltar
Err no submit solta o buffer sob DMA (F208). Fate forall
sobre o corpo extraído (`submit_complete_act`). O mutante
AS-IS (`submit_complete_act_as_is`) volta Err no submit
com CQE pendente — a planta DST
`harvest_on_submit_err_uses_cqe` recusa. -/
theorem submit_complete_act_fate_iff :
    ∀ (submit_ok harvested : Bool) (act : SubmitCompleteAct),
      (submit_complete_act submit_ok harvested = ok act) ↔
        ((harvested = true ∧ act = SubmitCompleteAct.UseHarvested)
          ∨ (harvested = false ∧
              act = SubmitCompleteAct.WaitMore)) := by
  intro submit_ok harvested act
  cases harvested with
  | true =>
      have key : submit_complete_act submit_ok true
          = ok SubmitCompleteAct.UseHarvested := by
        unfold submit_complete_act
        rfl
      rw [key]
      constructor
      · intro he
        exact Or.inl ⟨rfl, (Result.ok.inj he).symm⟩
      · intro hdisj
        cases hdisj with
        | inl hh =>
            rw [hh.2]
        | inr hh =>
            exact Bool.noConfusion hh.1
  | false =>
      have key : submit_complete_act submit_ok false
          = ok SubmitCompleteAct.WaitMore := by
        unfold submit_complete_act
        rfl
      rw [key]
      constructor
      · intro he
        exact Or.inr ⟨rfl, (Result.ok.inj he).symm⟩
      · intro hdisj
        cases hdisj with
        | inl hh =>
            exact Bool.noConfusion hh.1
        | inr hh =>
            rw [hh.2]
