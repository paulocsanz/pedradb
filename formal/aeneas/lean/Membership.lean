-- Theorems over Aeneas extract of raft membership_kernel.rs (Raft §6).
-- elect_claim_banner &'static str bottoms patched to toStr in aeneas_membership.sh.
import Aeneas
import MembershipKernel
open Aeneas.Std Result
open pedra_aeneas_membership_kernel

/-- Catalog entry: C-old majority is not enough during joint add. -/
theorem joint_election_ok_needs_both :
    joint_election_ok (2#u64) (3#u64) (some (2#u64, 4#u64)) = ok false := by
  unfold joint_election_ok
  unfold majority_of
  rfl

/-- AS-IS dente: C-old majority elects during joint add. -/
theorem joint_election_ok_as_is_dente :
    joint_election_ok_as_is (2#u64) (3#u64) (some (2#u64, 4#u64)) = ok true := by
  unfold joint_election_ok_as_is
  unfold majority_of
  rfl

/-- RFC-0069 P2.2: bounded elect does not print live. -/
theorem elect_claim_banner_bounded :
    elect_claim_banner false false false =
      ok (toStr "bounded-elect not-eventual") := by
  unfold elect_claim_banner
  unfold liveness_admitted
  simp

/-- AS-IS dente: banner is live without naming ES. -/
theorem elect_claim_banner_as_is_dente :
    elect_claim_banner_as_is false false false = ok (toStr "live") := by
  unfold elect_claim_banner_as_is
  simp

/-! ## RFC-0191 P1.4 — C1 close: ∀ contagens + Option joint -/

/-- Maioria como função pura (a mesma conta do kernel `majority_of`:
1 quando a configuração está vazia, `n/2+1` senão), fora do monoide
`Result`, para servir de enunciado fechado. -/
def maj (n : U64) : U64 :=
  if n = 0#u64 then 1#u64 else UScalar.mk (n.bv / 2#64 + 1#64)

private theorem ge_true_of_not_lt {x y : U64} (h : ¬ x < y) :
    ((x >= y) : Bool) = true := by
  have hn : ¬ (x.val < y.val) := by simpa using h
  simp only [decide_eq_true_eq, ge_iff_le, UScalar.le_equiv]
  omega

private theorem ge_false_of_lt {x y : U64} (h : x < y) :
    ((x >= y) : Bool) = false := by
  have hn : x.val < y.val := by simpa using h
  simp only [decide_eq_false_iff_not, ge_iff_le, UScalar.le_equiv]
  omega

private theorem add_one_ne (i : U64) (h : i.val + (1#u64).val ≤ U64.max) :
    i + 1#u64 = ok (UScalar.mk (i.bv + 1#64) : U64) := by
  obtain ⟨z, hz, -, hbv⟩ :=
    Aeneas.Std.WP.spec_imp_exists (U64.add_bv_spec h)
  rw [hz]
  have hbv' : z.bv = i.bv + 1#64 := hbv
  cases z; simp_all

/-- O kernel `majority_of` é total e coincide com a forma pura `maj`. -/
theorem majority_of_closed (n : U64) :
    majority_of n = ok (maj n) := by
  unfold majority_of
  split
  · rename_i h0
    show ok 1#u64 = ok (maj n)
    unfold maj
    rw [if_pos h0]
  · rename_i h0
    obtain ⟨z, hz, hzval, hbv⟩ :=
      UScalar.div_bv_spec n (y := 2#u64) (by decide)
    rw [hz]
    simp only [bind_tc_ok]
    have hbound : z.val + (1#u64).val ≤ U64.max := by
      rw [U64.max_eq]
      have h1 : (1#u64).val = 1 := by rfl
      have h2 := U64.lt_succ_max n
      have h3 : z.val = n.val / 2 := by rw [hzval]; rfl
      rw [h3, h1]
      omega
    rw [add_one_ne z hbound]
    unfold maj
    rw [if_neg h0]
    have hbv' : U64.bv z = n.bv / 2#64 := hbv
    rw [hbv']

/-- C1 (RFC-0191 P1.4, close): para todas as contagens de votos e todo o
`Option` do joint, a eleição conjunta devolve `ok true` exatamente quando
C-old tem maioria E (não há joint pendente OU C-new também tem maioria).
Não é o corpo do kernel re-afirmado: o lado direito é a especificação
fechada sobre a maioria pura `maj`. -/
theorem c1_joint_election :
    ∀ (old_yes old_n : U64) (new_yes : Option (U64 × U64)),
      joint_election_ok old_yes old_n new_yes =
        ok ((old_yes >= maj old_n) &&
            (match new_yes with
             | none => true
             | some p => p.1 >= maj p.2)) := by
  intro old_yes old_n new_yes
  unfold joint_election_ok
  rw [majority_of_closed]
  simp only [bind_tc_ok]
  split
  · rename_i hlt
    rw [ge_false_of_lt hlt, Bool.false_and]
  · rename_i hge
    rw [ge_true_of_not_lt hge, Bool.true_and]
    cases new_yes with
    | none => rfl
    | some p =>
      obtain ⟨yes, hn⟩ := p
      simp only [majority_of_closed, bind_tc_ok]
      rfl

/-- RFC-0191 P1.4, corolário: maioria de C-old sozinha NÃO elege
enquanto o joint está pendente — falta a maioria de C-new, ∀ contagens. -/
theorem c1_old_majority_alone_refuses :
    ∀ (old_yes old_n yes n : U64),
      ¬ (old_yes < maj old_n) → (yes < maj n) →
        joint_election_ok old_yes old_n (some (yes, n)) = ok false := by
  intro old_yes old_n yes n hold hnew
  rw [c1_joint_election]
  show ok ((old_yes >= maj old_n) && (yes >= maj n)) = ok false
  rw [ge_true_of_not_lt hold, ge_false_of_lt hnew, Bool.true_and]

/-- RFC-0191 P1.4, corolário: ambas as maiorias elegem, ∀ contagens. -/
theorem c1_both_majorities_elect :
    ∀ (old_yes old_n yes n : U64),
      ¬ (old_yes < maj old_n) → ¬ (yes < maj n) →
        joint_election_ok old_yes old_n (some (yes, n)) = ok true := by
  intro old_yes old_n yes n hold hnew
  rw [c1_joint_election]
  show ok ((old_yes >= maj old_n) && (yes >= maj n)) = ok true
  rw [ge_true_of_not_lt hold, ge_true_of_not_lt hnew, Bool.true_and]

/-- AS-IS ∀ (RFC-0191 P1.4): durante o joint, o mutante elege com a
maioria de C-old sozinha — C-new nunca é consultado, qualquer que seja
o `Option` do joint. -/
theorem c1_as_is_elects_on_old_alone :
    ∀ (old_yes old_n : U64) (new_yes : Option (U64 × U64)),
      joint_election_ok_as_is old_yes old_n new_yes =
        ok ((old_yes >= maj old_n) : Bool) := by
  intro old_yes old_n new_yes
  unfold joint_election_ok_as_is
  rw [majority_of_closed]
  simp only [bind_tc_ok]

/-- Catalog entry: an uncommitted leave finishes exactly when the leave
    entry is not in the log, or it is already committed
    (RFC-0122/0123 — a leave still in the log and uncommitted does not
    finish). -/
theorem queued_leave_finish_ok_iff_not_in_log_or_committed :
    ∀ (leave_in_log : Bool) (leave_committed : Bool),
      (queued_leave_finish_ok leave_in_log leave_committed = ok true)
        ↔ (leave_in_log = false ∨ leave_committed = true) := by
  intro leave_in_log leave_committed
  unfold queued_leave_finish_ok
  constructor
  · intro h
    split at h
    · next c1 =>
      simp at h
      exact Or.inr h
    · next c1 => exact Or.inl (by simp at c1; exact c1)
  · rintro (h1 | hc)
    · rw [if_neg (by simp [h1])]
    · split
      · next _ => rw [hc]
      · rfl

/-- Catalog entry: an election grant from a node counts exactly when
    the node is in the id set, or it is in the pending old-or-new joint
    set (RFC-0114/0116 — a node in neither set never grants). -/
theorem election_grant_from_counts_ok_iff_ids_or_pending :
    ∀ (in_ids : Bool) (in_pending_old_or_new : Bool),
      (election_grant_from_counts in_ids in_pending_old_or_new = ok true)
        ↔ (in_ids = true ∨ in_pending_old_or_new = true) := by
  intro in_ids in_pending
  unfold election_grant_from_counts
  constructor
  · intro h
    split at h
    · next c1 =>
      exact Or.inl c1
    · next c1 =>
      simp at h
      exact Or.inr h
  · rintro (hc | hc)
    · rw [if_pos hc]
    · split
      · next _ => rfl
      · next _ => rw [hc]

/-- Catalog entry: the joint election elects exactly when C-old has
    majority AND (no joint is pending, or C-new also has majority)
    (F-L28/RFC-0064 — one side alone never elects). -/
theorem joint_election_ok_elects_iff_old_and_new_majority :
    ∀ (old_yes old_n : U64) (new_yes : Option (U64 × U64)),
      (joint_election_ok old_yes old_n new_yes = ok true)
        ↔ (((old_yes >= maj old_n) &&
            (match new_yes with
             | none => true
             | some p => p.1 >= maj p.2)) : Bool) = true := by
  intro old_yes old_n new_yes
  rw [c1_joint_election]
  simp

/-- RFC-0205 P1.2 (first data-fate cadence promotion, atom
    `catalog:recover_apply`): recovery re-applies EXACTLY when the
    commit index is beyond applied (a committed entry not yet applied
    is never skipped, and an already-applied one is never replayed) —
    the fate forall over the extracted pure-lift body; the AS-IS
    `ok false` mutant skips everything. -/
theorem recover_must_apply_fate_iff :
    ∀ (applied : U64) (commit : U64) (v : Bool),
      (recover_must_apply applied commit = ok v) ↔
        ((v = true ∧ commit > applied)
          ∨ (v = false ∧ ¬(commit > applied))) := by
  intro applied commit v
  unfold recover_must_apply
  cases hd : decide (commit > applied) with
  | true =>
    have hP := of_decide_eq_true hd
    cases v <;> simp [hP]
  | false =>
    have hnP := of_decide_eq_false hd
    cases v <;> simp [hnP]

/-- RFC-0205 P1.2 (second data-fate cadence promotion, atom
    `catalog:recover_drop_orphan`): recovery drops an orphan segment
    EXACTLY when its index is beyond the new inventory high-water (a
    segment outside the committed inventory is never kept; one inside
    is never dropped) — fate forall over the extracted pure-lift body;
    the AS-IS `ok false` mutant keeps every orphan. -/
theorem recover_drop_orphan_seg_fate_iff :
    ∀ (seg_index : U64) (new_hi : U64) (v : Bool),
      (recover_drop_orphan_seg seg_index new_hi = ok v) ↔
        ((v = true ∧ seg_index > new_hi)
          ∨ (v = false ∧ ¬(seg_index > new_hi))) := by
  intro seg_index new_hi v
  unfold recover_drop_orphan_seg
  cases hd : decide (seg_index > new_hi) with
  | true =>
    have hP := of_decide_eq_true hd
    cases v <;> simp [hP]
  | false =>
    have hnP := of_decide_eq_false hd
    cases v <;> simp [hnP]
