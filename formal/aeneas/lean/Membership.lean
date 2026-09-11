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

/-- RFC-0208 P1.2 (membership cadence promotion 1/4, atom
    `catalog:removed_step_down`): a node removed from the committed
    membership steps down EXACTLY when its id left the id set — the
    node that stayed never steps down, the removed one always does —
    fate forall over the extracted pure-lift body; the AS-IS
    `ok false` mutant never steps down (a removed leader keeps
    leading). -/
theorem removed_steps_down_fate_iff :
    ∀ (in_ids : Bool) (v : Bool),
      (removed_steps_down in_ids = ok v) ↔
        ((v = true ∧ ¬ (in_ids = true))
          ∨ (v = false ∧ in_ids = true)) := by
  intro in_ids v
  unfold removed_steps_down
  cases in_ids <;> cases v <;> simp

/-- RFC-0208 P1.2 (membership cadence promotion 2/4, atom
    `catalog:disk_membership`): the cluster identity bound at reopen
    is the DISK one EXACTLY when a disk membership exists — the CLI
    flag never overrides a persisted membership — fate forall over
    the extracted pure-lift body; the AS-IS `ok false` mutant always
    lets the CLI win (split-brain on reopen). -/
theorem disk_membership_overrides_cli_fate_iff :
    ∀ (has_disk : Bool) (v : Bool),
      (disk_membership_overrides_cli has_disk = ok v) ↔
        ((v = true ∧ has_disk = true)
          ∨ (v = false ∧ has_disk = false)) := by
  intro has_disk v
  unfold disk_membership_overrides_cli
  cases has_disk <;> cases v <;> simp

/-- RFC-0208 P1.2 (membership cadence promotion 3/4, atom
    `catalog:high_water`): the inventory high-water after reopen is
    the MAX of disk and ram EXACTLY — the trait-default `Ord::max`
    over the U64 order — fate forall over the extracted body; the
    AS-IS mutant keeps the ram value and can LOSE committed
    inventory (a durable high-water below the in-memory one). -/
theorem high_water_at_least_fate_iff :
    ∀ (disk_hw ram_hw : U64) (v : U64),
      (high_water_at_least disk_hw ram_hw = ok v) ↔
        ((v = ram_hw ∧ disk_hw < ram_hw)
          ∨ (v = disk_hw ∧ ¬ (disk_hw < ram_hw))) := by
  intro disk_hw ram_hw v
  have hsem : ∀ (x y : U64),
      core.cmp.OrdU64.partialOrdInst.lt x y = ok (decide (x < y)) := fun x y => rfl
  unfold high_water_at_least core.cmp.Ord.max.default core.cmp.Ord.max_body
  rw [hsem]
  cases hd : decide (disk_hw < ram_hw) with
  | true =>
    have hP : disk_hw < ram_hw := of_decide_eq_true hd
    simp [hd, hP]
    exact eq_comm
  | false =>
    have hnP : ¬ (disk_hw < ram_hw) := of_decide_eq_false hd
    simp [hd, hnP]
    exact eq_comm

/-- RFC-0208 P1.2 (membership cadence promotion 4/4, atom
    `catalog:joint_leave`): the joint configuration is still active
    EXACTLY when the old and new id slices DIFFER (elementwise
    U64 equality) — a joint that already converged to the new set is
    gone — fate forall over the extracted body, bridged through the
    Aeneas spec theorems (`PartialEqSlice.eq_homo_spec` +
    `spec_imp_exists`; the scalar `ne` is a pure lift); the AS-IS
    `ok false` mutant declares every joint dead on sight. -/
theorem joint_still_active_fate_iff :
    ∀ (old new : Slice U64) (v : Bool),
      (joint_still_active old new = ok v) ↔
        ((v = true ∧ old ≠ new)
          ∨ (v = false ∧ old = new)) := by
  intro old new v
  have hNe : ∀ (x y : U64), WP.spec (core.cmp.PartialEqU64.ne x y)
      (fun b => b ↔ ¬ (x = y)) := by
    intro x y
    simp only [core.cmp.PartialEqU64, liftFun2]
    exact (WP.spec_ok _).2 (by simp)
  have heq := core.slice.cmp.PartialEqSlice.eq_homo_spec
    core.cmp.PartialEqU64 old new hNe
  obtain ⟨beq, rheq, hbeq⟩ := WP.spec_imp_exists heq
  unfold joint_still_active
  simp only [core.cmp.impls.PartialEqShared.ne,
             Slice.Insts.CoreCmpPartialEqSlice,
             core.cmp.PartialEq.ne.trait_default,
             core.cmp.PartialEq.ne.default]
  rw [rheq]
  cases beq with
  | true =>
    have he : old = new := hbeq.mp rfl
    cases v <;> simp [he]
  | false =>
    have hne : ¬ (old = new) := fun h => absurd (hbeq.mpr h) (by simp)
    cases v <;> simp [hne]

/-- RFC-0212 P0.1 (membership cadence 1/6, atom
    `catalog:discard_uncommitted`): the live uncommitted-log
    discard runs on EVERY local replica EXACTLY when the node is
    local — membership in `ids` is not the gate (a replica dropped
    from `ids` still discards its uncommitted suffix) — fate
    forall over the extracted pure-lift body; the AS-IS mutant
    gates on `is_local && in_ids` (the 0142 leftover: the removed
    replica keeps the suffix — the lie the DST plant
    `discard_node_counts_on_live_queued_is_not_ok` refutes). -/
theorem discard_node_counts_fate_iff :
    ∀ (is_local in_ids : Bool) (v : Bool),
      (discard_node_counts is_local in_ids = ok v) ↔
        ((v = true ∧ is_local = true)
          ∨ (v = false ∧ is_local = false)) := by
  intro is_local in_ids v
  unfold discard_node_counts
  cases is_local <;> cases v <;> simp

/-- RFC-0212 P0.1 (membership cadence 1/6, atom
    `catalog:discard_leader`): the no-leader discard
    persist-leader is a LOCAL node EXACTLY when the chosen node is
    local — `next_index` repair runs where the persist lands —
    fate forall over the extracted pure-lift body; the AS-IS
    `ok true` mutant accepts `ids.first()` even when remote (the
    0143 leftover — the lie the DST plant
    `discard_leader_local_on_live_queued_is_not_ok` refutes). -/
theorem discard_leader_local_fate_iff :
    ∀ (is_local : Bool) (v : Bool),
      (discard_leader_local is_local = ok v) ↔
        ((v = true ∧ is_local = true)
          ∨ (v = false ∧ is_local = false)) := by
  intro is_local v
  unfold discard_leader_local
  cases is_local <;> cases v <;> simp

/-- RFC-0212 P0.1 (membership cadence 1/6, atom
    `catalog:drop_preimages`): prepare-time preimages are dropped
    on EVERY local replica EXACTLY when the node is local —
    membership in `ids` is not the gate (a replica dropped from
    `ids` still drops its preimages) — fate forall over the
    extracted pure-lift body; the AS-IS mutant gates on
    `is_local && in_ids` (the 0138 leftover: the removed replica
    keeps preimages — the lie the DST plant
    `drop_preimages_node_counts_on_live_queued_is_not_ok`
    refutes). -/
theorem drop_preimages_node_counts_fate_iff :
    ∀ (is_local in_ids : Bool) (v : Bool),
      (drop_preimages_node_counts is_local in_ids = ok v) ↔
        ((v = true ∧ is_local = true)
          ∨ (v = false ∧ is_local = false)) := by
  intro is_local in_ids v
  unfold drop_preimages_node_counts
  cases is_local <;> cases v <;> simp

/-- RFC-0212 P0.1 (membership cadence 1/6, atom
    `catalog:force_clear`): the force-local TX clear runs on EVERY
    local replica EXACTLY when the node is local — membership in
    `ids` is not the gate (a replica dropped from `ids` still
    clears its stuck intents) — fate forall over the extracted
    pure-lift body; the AS-IS mutant gates on
    `is_local && in_ids` (the 0137 leftover: the removed replica
    keeps intents — the lie the DST plant
    `force_clear_node_counts_on_live_queued_is_not_ok` refutes). -/
theorem force_clear_node_counts_fate_iff :
    ∀ (is_local in_ids : Bool) (v : Bool),
      (force_clear_node_counts is_local in_ids = ok v) ↔
        ((v = true ∧ is_local = true)
          ∨ (v = false ∧ is_local = false)) := by
  intro is_local in_ids v
  unfold force_clear_node_counts
  cases is_local <;> cases v <;> simp

/-- RFC-0212 P0.2 (membership cadence 2/6, atom
    `catalog:persist_meta`): SI meta is persisted on EVERY local
    replica EXACTLY when the node is local — membership in `ids`
    is not the gate (a replica dropped from `ids` still persists
    its clock) — fate forall over the extracted pure-lift body;
    the AS-IS mutant gates on `is_local && in_ids` (the 0134
    leftover: the removed replica's clock is not durable — the
    lie the DST plant
    `persist_meta_node_counts_on_live_queued_is_not_ok` refutes). -/
theorem persist_meta_node_counts_fate_iff :
    ∀ (is_local in_ids : Bool) (v : Bool),
      (persist_meta_node_counts is_local in_ids = ok v) ↔
        ((v = true ∧ is_local = true)
          ∨ (v = false ∧ is_local = false)) := by
  intro is_local in_ids v
  unfold persist_meta_node_counts
  cases is_local <;> cases v <;> simp
