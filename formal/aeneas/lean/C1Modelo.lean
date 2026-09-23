-- Theorems over Aeneas extract of c1_modelo_kernel.rs (RFC-0166 P2.3).
import Aeneas
import C1ModeloKernel
open Aeneas.Std Result
open pedra_aeneas_c1_modelo_kernel

private def jointAdd : c1_modelo_kernel.C1State :=
  { old_n := 3#u64, old_yes := 2#u64, joint := true, new_n := 4#u64,
    new_yes := 1#u64, current_term := 7#u64, index_term := 7#u64,
    commit_index := 0#u64, proposed := 5#u64, served := true }

/-- Catalog entry: joint-add shape does not serve. -/
theorem c1_modelo_joint_add_refuses :
    c1_modelo_kernel.c1_modelo jointAdd = ok false := by
  unfold c1_modelo_kernel.c1_modelo
  unfold c1_modelo_kernel.c1_advance_commit
  unfold c1_modelo_kernel.c1_quorum
  unfold c1_modelo_kernel.new_cfg
  unfold membership_kernel.joint_election_ok
  unfold membership_kernel.majority_of
  unfold commit_kernel.may_commit_at
  unfold commit_kernel.propose_ack_ok
  rfl

/-- AS-IS dente: C-old majority acks during joint add. -/
theorem c1_modelo_as_is_dente :
    c1_modelo_kernel.c1_modelo_as_is jointAdd = ok true := by
  unfold c1_modelo_kernel.c1_modelo_as_is
  unfold c1_modelo_kernel.c1_advance_commit_as_is
  unfold c1_modelo_kernel.new_cfg
  unfold membership_kernel.joint_election_ok_as_is
  unfold membership_kernel.majority_of
  unfold commit_kernel.may_commit_at_as_is
  unfold commit_kernel.propose_ack_ok_as_is
  rfl

/-! ## RFC-0215 P0.2 — coroa de produto no degrau átomo (modelo ×4) -/

/-- Any ok-valued Result bind forces the bound term to be ok
(Cf.lean's `bind_ok_inv`, restated for this module). -/
private theorem bind_ok_inv {α β} (x : Result α) (f : α → Result β) (v : β)
    (h : Aeneas.Std.bind x f = ok v) : ∃ a, x = ok a ∧ f a = ok v := by
  cases x with
  | ok a => exact ⟨a, rfl, h⟩
  | fail e => exact absurd h (by simp)
  | div => exact absurd h (by simp)

/-- C1 modelo mantém: não serve, ou o ack proposto é legítimo
(`c1_advance_commit`/`propose_ack_ok` citados, corpos não reabertos). -/
def c1m_ok_true (s : c1_modelo_kernel.C1State) : Prop :=
  ∃ t, c1_modelo_kernel.c1_advance_commit s = ok t ∧
    (t.served = false ∨ t.served = true ∧
      commit_kernel.propose_ack_ok t.proposed t.commit_index = ok true)

/-- C1 modelo falha: serve um ack que o commit não sustenta. -/
def c1m_acks_uncommitted (s : c1_modelo_kernel.C1State) : Prop :=
  ∃ t, c1_modelo_kernel.c1_advance_commit s = ok t ∧ t.served = true ∧
    commit_kernel.propose_ack_ok t.proposed t.commit_index = ok false

/-- RFC-0215 P0.2 4/4 (atom `catalog:c1_modelo`, entry `c1_modelo`):
o desfecho da máquina C1 é exatamente a decisão que o spec nomeia —
`ok false` somente quando o modelo serve um ack que o commit não
sustenta (o buraco do mutante AS-IS, que aceita qualquer ack);
`ok true` pelos demais caminhos ok. -/
theorem c1_modelo_fate_iff :
    ∀ (s : c1_modelo_kernel.C1State) (v : Bool),
      (c1_modelo_kernel.c1_modelo s = ok v) ↔
        ((v = true ∧ c1m_ok_true s) ∨
          (v = false ∧ c1m_acks_uncommitted s)) := by
  intro s v
  constructor
  · intro hval
    unfold c1_modelo_kernel.c1_modelo at hval
    obtain ⟨ t, ht, hval ⟩ := bind_ok_inv _ _ _ hval
    split at hval
    · next hserved =>
        cases v with
        | true =>
            exact Or.inl ⟨rfl, ⟨t, ht, Or.inr ⟨hserved, hval⟩⟩⟩
        | false =>
            exact Or.inr ⟨rfl, ⟨t, ht, hserved, hval⟩⟩
    · next hns =>
        have hsf : t.served = false := by simpa [Bool.not_eq_true] using hns
        have hv : v = true := (Result.ok.inj hval).symm
        exact Or.inl ⟨hv, ⟨t, ht, Or.inl hsf⟩⟩
  · intro hdisj
    cases hdisj with
    | inl hh =>
        obtain ⟨hv, t, ht, hbr⟩ := hh
        subst hv
        unfold c1_modelo_kernel.c1_modelo
        rw [ht]
        simp only [Aeneas.Std.bind_tc_ok]
        rcases hbr with hsf | ⟨hserved, hcall⟩
        · rw [hsf, if_neg (by simp)]
        · rw [hserved, if_pos rfl]
          exact hcall
    | inr hh =>
        obtain ⟨hv, t, ht, hserved, hcall⟩ := hh
        subst hv
        unfold c1_modelo_kernel.c1_modelo
        rw [ht]
        simp only [Aeneas.Std.bind_tc_ok]
        rw [hserved, if_pos rfl]
        exact hcall

/-! ## RFC-0215 P1.1 — coroa de produto no degrau átomo (fate ×2) -/

/-- RFC-0215 P1.1 2/2 (atom `catalog:c1_advance_commit`, entry
`c1_advance_commit`): o commit avança exatamente na maioria — `ok t` é
exatamente: `c1_quorum` computa `b`, `may_commit_at` decide `b1`; sem
maioria `t = s`, com maioria `t` leva o máximo proposto
(`c1_quorum`/`may_commit_at`/`Ord.max` citados, corpos não reabertos).
O mutante AS-IS (`c1_advance_commit_as_is`) aceita commit sem maioria;
planta três-dentes recusa. -/
theorem c1_advance_commit_fate_iff :
    ∀ (s t : c1_modelo_kernel.C1State),
      (c1_modelo_kernel.c1_advance_commit s = ok t) ↔
        ∃ b, c1_modelo_kernel.c1_quorum s = ok b ∧
          ∃ b1, commit_kernel.may_commit_at
              s.index_term s.current_term b = ok b1 ∧
            ((b1 = false ∧ t = s) ∨
              (b1 = true ∧
                ∃ i, core.cmp.Ord.max.default
                    core.cmp.OrdU64.partialOrdInst.lt
                    s.proposed s.commit_index = ok i ∧
                  t = { s with commit_index := i })) := by
  intro s t
  constructor
  · intro hval
    unfold c1_modelo_kernel.c1_advance_commit at hval
    obtain ⟨ b, hb, hval ⟩ := bind_ok_inv _ _ _ hval
    obtain ⟨ b1, hb1, hval ⟩ := bind_ok_inv _ _ _ hval
    split at hval
    · next hb1' =>
        obtain ⟨ i, hi, hval ⟩ := bind_ok_inv _ _ _ hval
        have ht : t = { s with commit_index := i } :=
          (Result.ok.inj hval).symm
        exact ⟨ b, hb, b1, hb1, Or.inr ⟨hb1', i, hi, ht⟩⟩
    · next hb1n =>
        have hb1F : b1 = false := by simpa [Bool.not_eq_true] using hb1n
        have ht : t = s := (Result.ok.inj hval).symm
        exact ⟨ b, hb, b1, hb1, Or.inl ⟨hb1F, ht⟩⟩
  · rintro ⟨ b, hb, b1, hb1, hbr ⟩
    unfold c1_modelo_kernel.c1_advance_commit
    rw [hb]
    simp only [Aeneas.Std.bind_tc_ok]
    rw [hb1]
    simp only [Aeneas.Std.bind_tc_ok]
    rcases hbr with ⟨hb1F, ht⟩ | ⟨hb1T, i, hi, ht⟩
    · rw [hb1F, if_neg (by simp), ht]
    · rw [hb1T, if_pos rfl]
      rw [hi]
      simp only [Aeneas.Std.bind_tc_ok]
      rw [ht]
