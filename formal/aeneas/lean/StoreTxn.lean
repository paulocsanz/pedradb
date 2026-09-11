-- Theorems over the Aeneas extract of production store txn_kernel.rs
-- (F47/F34/F52/F35/F36/F49/F50). Payment is the linked rustc bodies; the
-- former cfg(verus_keep_ghost) stand-in was deleted (RFC-0171 P0.3).
-- Fail-closed: this file must not contain a hole.
import Aeneas
import StoreTxnKernel
open Aeneas.Std Result
open pedra_aeneas_store_txn_kernel

/-- F47 teeth: a fenced abort never materialises. -/
theorem txn_commit_action_abort_reverts :
    txn_commit_action true = ok TxnCommitAction.Revert := by
  unfold txn_commit_action
  rfl

/-- F47: a commit materialises. -/
theorem txn_commit_action_commit_materialises :
    txn_commit_action false = ok TxnCommitAction.Materialise := by
  unfold txn_commit_action
  rfl

/-- AS-IS F47 dente: abort still materialises (heal installs the aborted TX). -/
theorem txn_commit_action_as_is_abort_materialises :
    txn_commit_action_as_is true = ok TxnCommitAction.Materialise := by
  unfold txn_commit_action_as_is
  rfl

/-- F47: abort keeps the status fence even with pairs gone. -/
theorem revert_clears_status_abort_keeps_fence :
    revert_clears_status true true = ok false := by
  unfold revert_clears_status
  rfl

/-- F47: non-abort with pairs gone may clear. -/
theorem revert_clears_status_commit_clears :
    revert_clears_status false true = ok true := by
  unfold revert_clears_status
  rfl

/-- AS-IS F47 dente: the fence evaporates on abort. -/
theorem revert_clears_status_as_is_drops_fence :
    revert_clears_status_as_is true true = ok true := by
  unfold revert_clears_status_as_is
  rfl

/-- F47: the cut never lands at or below a committed index. -/
theorem discard_cut_never_cuts_committed :
    discard_cut (3#u64) (5#u64) = ok 6#u64 := by
  unfold discard_cut
  have h : core.num.U64.saturating_add 5#u64 1#u64 = 6#u64 := by native_decide
  simp [h, lift, core.cmp.Ord.max.default,
    core.cmp.Ord.max_body, core.cmp.impls.PartialOrdU64.lt]

/-- F47: a cut already above commit stays. -/
theorem discard_cut_keeps_higher_from :
    discard_cut (8#u64) (5#u64) = ok 8#u64 := by
  unfold discard_cut
  have h : core.num.U64.saturating_add 5#u64 1#u64 = 6#u64 := by native_decide
  simp [h, lift, core.cmp.Ord.max.default,
    core.cmp.Ord.max_body, core.cmp.impls.PartialOrdU64.lt]

/-- AS-IS F47 dente: the cut can land on a committed index. -/
theorem discard_cut_as_is_cuts_committed :
    discard_cut_as_is (3#u64) (5#u64) = ok 3#u64 := by
  unfold discard_cut_as_is
  rfl

/-- F34 teeth: missing preimage record leaves the user key untouched. -/
theorem revert_user_action_missing_pre_leaves_key :
    revert_user_action false false = ok RevertUserAction.LeaveUntouched := by
  unfold revert_user_action
  rfl

/-- F34: absent preimage restores absent. -/
theorem revert_user_action_absent_restores_absent :
    revert_user_action true true = ok RevertUserAction.RestoreAbsent := by
  unfold revert_user_action
  rfl

/-- F34: value preimage restores the value. -/
theorem revert_user_action_value_restores_value :
    revert_user_action true false = ok RevertUserAction.RestoreValue := by
  unfold revert_user_action
  rfl

/-- AS-IS F34 dente: always blind-deletes. -/
theorem revert_user_action_as_is_blind_deletes :
    revert_user_action_as_is false false = ok RevertUserAction.RestoreAbsent := by
  unfold revert_user_action_as_is
  rfl

/-- F52: hist repair only for a restored user key that is not reserved. -/
theorem should_repair_si_hist_only_restored_unreserved :
    should_repair_si_hist true false = ok true
      ∧ should_repair_si_hist true true = ok false := by
  unfold should_repair_si_hist
  simp

/-- AS-IS F52 dente: hist never repaired. -/
theorem should_repair_si_hist_as_is_never :
    should_repair_si_hist_as_is true false = ok false := by
  unfold should_repair_si_hist_as_is
  rfl

/-- F35 teeth: leftover prepared TX after crash is aborted. -/
theorem leftover_txn_is_aborted_true :
    leftover_txn_is_aborted = ok true := by
  unfold leftover_txn_is_aborted leftover_fate
  rfl

/-- AS-IS F35 dente: intents stay live (immortal Conflict). -/
theorem leftover_txn_is_aborted_as_is_false :
    leftover_txn_is_aborted_as_is = ok false := by
  unfold leftover_txn_is_aborted_as_is leftover_fate_as_is
  rfl

/-- F35: the next id never reuses the durable max. -/
theorem next_txn_id_after_advances :
    next_txn_id_after (7#u64) = ok 8#u64 := by
  unfold next_txn_id_after
  have h : core.num.U64.saturating_add 7#u64 1#u64 = 8#u64 := by native_decide
  simp [h, lift, core.cmp.Ord.max.default,
    core.cmp.Ord.max_body, core.cmp.impls.PartialOrdU64.lt]

/-- F35: floor at 1 when the durable counter is empty. -/
theorem next_txn_id_after_floor_one :
    next_txn_id_after (0#u64) = ok 1#u64 := by
  unfold next_txn_id_after
  have h : core.num.U64.saturating_add 0#u64 1#u64 = 1#u64 := by native_decide
  simp [h, lift, core.cmp.Ord.max.default,
    core.cmp.Ord.max_body, core.cmp.impls.PartialOrdU64.lt]

/-- F36 teeth: SI generation comes from the durable max. -/
theorem recover_si_generation_survives :
    recover_si_generation (9#u64) = ok 9#u64 := by
  rfl

/-- AS-IS F36 dente: generation evaporates on reopen. -/
theorem recover_si_generation_as_is_evaporates :
    recover_si_generation_as_is (9#u64) = ok 0#u64 := by
  unfold recover_si_generation_as_is
  rfl

/-- F50 teeth: a failed prepare aborts already-durable intents. -/
theorem prepare_error_aborts_earlier_true :
    prepare_error_aborts_earlier = ok true := by
  unfold prepare_error_aborts_earlier
  rfl

/-- AS-IS F50 dente: `?` on NotLeader leaves intents live. -/
theorem prepare_error_aborts_earlier_as_is_false :
    prepare_error_aborts_earlier_as_is = ok false := by
  unfold prepare_error_aborts_earlier_as_is
  rfl

/-- F49 teeth: the reserve advances the counter and stamps that value. -/
theorem reserve_si_gen_advances_and_differs :
    reserve_si_gen (3#u64) = ok { next_current := 4#u64, reserved := 4#u64 } := by
  unfold reserve_si_gen
  have h : core.num.U64.saturating_add 3#u64 1#u64 = 4#u64 := by native_decide
  simp [h, lift]

/-- AS-IS F49 dente: counter unmoved, next reserve collides. -/
theorem reserve_si_gen_as_is_collides :
    reserve_si_gen_as_is (3#u64) = ok { next_current := 3#u64, reserved := 4#u64 } := by
  unfold reserve_si_gen_as_is
  have h : core.num.U64.saturating_add 3#u64 1#u64 = 4#u64 := by native_decide
  simp [h, lift]

/-- F49: undo only when the counter still holds our reservation. -/
theorem unreserve_si_gen_only_if_still_ours :
    unreserve_si_gen (4#u64) (4#u64) = ok 3#u64
      ∧ unreserve_si_gen (5#u64) (4#u64) = ok 5#u64 := by
  unfold unreserve_si_gen
  have hpos : (4#u64 > 0#u64) = true := by native_decide
  have hdiff : (5#u64 = 4#u64) = false := by native_decide
  have hsub : core.num.U64.saturating_sub 4#u64 1#u64 = 3#u64 := by native_decide
  simp [hpos, hdiff, hsub]

/-- RFC-0212 P2.1 (store-txn cadence, atom `catalog:discard_cut`):
    the discard cut is EXACTLY max(from_index, commit ⊕ 1) under the
    U64 order — never at or below a committed index — fate forall
    over the extracted body (F47); the AS-IS mutant cuts at
    `from_index` even when committed (committed data loss — the
    lie the DST plant `discard_cut_on_live_queued_is_not_ok`
    refutes). -/
theorem discard_cut_fate_iff :
    ∀ (from_index commit v : U64),
      (discard_cut from_index commit = ok v) ↔
        ((v = core.num.U64.saturating_add commit 1#u64
            ∧ from_index < core.num.U64.saturating_add commit 1#u64)
          ∨ (v = from_index
            ∧ ¬ (from_index < core.num.U64.saturating_add commit 1#u64))) := by
  intro from_index commit v
  have hreduce : discard_cut from_index commit
      = core.cmp.Ord.max.default core.cmp.OrdU64.partialOrdInst.lt
          from_index (core.num.U64.saturating_add commit 1#u64) := rfl
  rw [hreduce]
  have hsem : ∀ (x y : U64),
      core.cmp.OrdU64.partialOrdInst.lt x y = ok (decide (x < y)) := fun x y => rfl
  unfold core.cmp.Ord.max.default core.cmp.Ord.max_body
  rw [hsem]
  cases hd : decide (from_index < core.num.U64.saturating_add commit 1#u64) with
  | true =>
    have hP : from_index < core.num.U64.saturating_add commit 1#u64 := of_decide_eq_true hd
    simp [hd, hP]
    exact eq_comm
  | false =>
    have hnP : ¬ (from_index < core.num.U64.saturating_add commit 1#u64) := of_decide_eq_false hd
    simp [hd, hnP]
    exact eq_comm

/-- RFC-0212 P2.1 (store-txn cadence, atom
    `catalog:leftover_txn_is_aborted`): a leftover prepared TX with
    no coordinator log is aborted — the uncommitted leftover's fate
    is EXACTLY true — fate forall over the extracted body (F35);
    the AS-IS mutant leaves intents live (immortal Conflict + id
    reuse — the lie the DST plant
    `leftover_txn_is_aborted_on_live_reopen_is_not_ok` refutes). -/
theorem leftover_txn_is_aborted_fate_iff :
    ∀ (v : Bool), (leftover_txn_is_aborted = ok v) ↔ v = true := by
  intro v
  unfold leftover_txn_is_aborted leftover_fate
  cases v <;> simp

/-- RFC-0212 P2.1 (store-txn cadence, atom `catalog:next_txn_id_after`):
    the next txn id is EXACTLY max(max_seen ⊕ 1, 1) — never a reuse
    of a durable id, floored at 1 — fate forall over the extracted
    body (F35); the AS-IS mutant restarts the counter at 1 (id
    collision with the leftover txn on disk — the lie the DST plant
    `next_txn_id_after_on_live_reopen_is_not_ok` refutes). -/
theorem next_txn_id_after_fate_iff :
    ∀ (max_seen v : U64),
      (next_txn_id_after max_seen = ok v) ↔
        ((v = 1#u64
            ∧ core.num.U64.saturating_add max_seen 1#u64 < 1#u64)
          ∨ (v = core.num.U64.saturating_add max_seen 1#u64
            ∧ ¬ (core.num.U64.saturating_add max_seen 1#u64 < 1#u64))) := by
  intro max_seen v
  have hreduce : next_txn_id_after max_seen
      = core.cmp.Ord.max.default core.cmp.OrdU64.partialOrdInst.lt
          (core.num.U64.saturating_add max_seen 1#u64) 1#u64 := rfl
  rw [hreduce]
  have hsem : ∀ (x y : U64),
      core.cmp.OrdU64.partialOrdInst.lt x y = ok (decide (x < y)) := fun x y => rfl
  unfold core.cmp.Ord.max.default core.cmp.Ord.max_body
  rw [hsem]
  cases hd : decide (core.num.U64.saturating_add max_seen 1#u64 < 1#u64) with
  | true =>
    have hP : core.num.U64.saturating_add max_seen 1#u64 < 1#u64 := of_decide_eq_true hd
    simp [hd, hP]
    exact eq_comm
  | false =>
    have hnP : ¬ (core.num.U64.saturating_add max_seen 1#u64 < 1#u64) := of_decide_eq_false hd
    simp [hd, hnP]
    exact eq_comm

/-- RFC-0212 P2.1 (store-txn cadence, atom
    `catalog:prepare_error_aborts_earlier`): a failed prepare step
    aborts the already-durable intents on earlier ranges — the
    decision is EXACTLY true — fate forall over the extracted body
    (F50); the AS-IS mutant `?`-returns without cleanup (immortal
    Conflict — the lie the DST plant
    `prepare_error_aborts_earlier_on_live_queued_is_not_ok`
    refutes). -/
theorem prepare_error_aborts_earlier_fate_iff :
    ∀ (v : Bool), (prepare_error_aborts_earlier = ok v) ↔ v = true := by
  intro v
  unfold prepare_error_aborts_earlier
  cases v <;> simp

/-- RFC-0212 P2.1 (store-txn cadence, atom `catalog:reserve_si_gen`):
    a reserve advances the counter AND stamps that value —
    `next_current` and `reserved` are EXACTLY the saturating
    successor of `current` — fate forall over the extracted body
    (F49: distinct outstanding gens); the AS-IS mutant leaves the
    counter unmoved (the next reserve collides — the lie the DST
    plant `reserve_si_gen_on_live_queued_is_not_ok` refutes). -/
theorem reserve_si_gen_fate_iff :
    ∀ (current : U64) (r : SiGenReserve),
      (reserve_si_gen current = ok r) ↔
        (r.next_current = core.num.U64.saturating_add current 1#u64
          ∧ r.reserved = core.num.U64.saturating_add current 1#u64) := by
  intro current r
  have hreduce : reserve_si_gen current
      = ok ({ next_current := core.num.U64.saturating_add current 1#u64
             , reserved := core.num.U64.saturating_add current 1#u64 :
             SiGenReserve }) := rfl
  rw [hreduce]
  cases r with
  | mk nc rs => simp [eq_comm]

