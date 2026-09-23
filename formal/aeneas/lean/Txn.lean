-- Theorems over Aeneas extract of store txn_kernel.rs (F47).
-- Ord.max.default patched to pass lt, not the Ord instance.
import Aeneas
import TxnKernel
open Aeneas.Std Result
open pedra_aeneas_txn_kernel

/-- Catalog entry: abort fence reverts, never materialises. -/
theorem txn_commit_action_abort_reverts :
    txn_commit_action true = ok TxnCommitAction.Revert := by
  unfold txn_commit_action
  rfl

/-- AS-IS dente: abort still materialises. -/
theorem txn_commit_action_as_is_dente :
    txn_commit_action_as_is true = ok TxnCommitAction.Materialise := by
  unfold txn_commit_action_as_is
  rfl

/-- Catalog corollary: leftover TX recovers aborted (T1 shim unfolds this). -/
theorem leftover_txn_is_aborted_true :
    leftover_txn_is_aborted = ok true := by
  unfold leftover_txn_is_aborted leftover_fate
  rfl

/-- AS-IS dente: leftover TX is not aborted. -/
theorem leftover_txn_is_aborted_as_is_dente :
    leftover_txn_is_aborted_as_is = ok false := by
  unfold leftover_txn_is_aborted_as_is leftover_fate_as_is
  rfl

/-- RFC-0191 P1.3 T1: leftover aborts iff not committed. Unfolds the
    rustc-linked `leftover_fate` over the whole Bool space — the
    constant `leftover_txn_is_aborted` does not pay this. -/
theorem t1_leftover_fate :
    ∀ (committed : Bool),
      leftover_fate committed = ok (!committed) := by
  intro committed
  unfold leftover_fate
  cases committed <;> rfl

/-- RFC-0191 P1.5 trampoline atom (F52/F117): the SI hist repair fate is
pinned by (tip_gen, tip_matches) — the gen-0 preimage floor never
rewrites and an already-matching tip never churns; every other tip
rewrites. The store trampoline (`repair_si_hist_tip`) matches this. -/
theorem si_hist_repair_plan_leave_iff_floor_or_match :
    ∀ (tip_gen : U64) (tip_matches : Bool),
      (si_hist_repair_plan tip_gen tip_matches = ok SiHistRepair.Leave)
        ↔ (tip_gen = 0#u64 ∨ tip_matches = true) := by
  intro tip_gen tip_matches
  unfold si_hist_repair_plan
  split <;> rename_i c
  · exact ⟨fun _ => Or.inl c, fun _ => rfl⟩
  · split <;> rename_i c2
    · exact ⟨fun _ => Or.inr c2, fun _ => rfl⟩
    · refine ⟨fun h => ?_, fun h => ?_⟩
      · exact absurd h (by simp)
      · rcases h with h0 | hm
        · exact absurd h0 c
        · exact absurd hm c2

/-- AS-IS dente: the repair stomps the gen-0 preimage floor. -/
theorem si_hist_repair_plan_as_is_dente :
    si_hist_repair_plan_as_is 0#u64 false = ok SiHistRepair.Rewrite := by
  unfold si_hist_repair_plan_as_is
  rfl

/-- RFC-0191 P2.3 cadence atom (third if, F119): the hist-load merge —
    a decoded hist merges only when its tip is not below the best-so-far
    tip; a corrupt hist never evicts a good copy. The store trampoline
    (`load_si_from_disk`) matches this. -/
theorem hist_load_fate_merge_new_iff_decoded_and_not_below :
    ∀ (decoded_ok : Bool) (best_has_user : Bool) (new_last : U64) (existing : U64),
      (hist_load_fate decoded_ok best_has_user new_last existing
          = ok HistLoadFate.MergeNew)
        ↔ (decoded_ok = true ∧ new_last >= existing) := by
  intro decoded_ok best_has_user new_last existing
  unfold hist_load_fate
  constructor
  · intro h
    split at h
    · next c1 =>
      split at h
      · next c2 => exact ⟨c1, c2⟩
      · next c2 => exact absurd h (by simp)
    · next c1 =>
      split at h
      · next c3 => exact absurd h (by simp)
      · next c3 => exact absurd h (by simp)
  · rintro ⟨hd, hm⟩
    rw [if_pos hd, if_pos hm]

/-- AS-IS dente: the corrupt replica wins (its hist replaces a newer
    best tip — F119). -/
theorem hist_load_fate_as_is_dente :
    hist_load_fate_as_is false true 0#u64 9#u64
      = ok HistLoadFate.MergeNew := by
  unfold hist_load_fate_as_is
  rfl

/-- Catalog entry: revert restores the prepare-time value exactly when a
    preimage record exists AND records the key as present (F34). -/
theorem revert_user_action_restore_value_iff_record_and_present :
    ∀ (had_pre_record : Bool) (pre_was_absent : Bool),
      (revert_user_action had_pre_record pre_was_absent
          = ok RevertUserAction.RestoreValue)
        ↔ (had_pre_record = true ∧ pre_was_absent = false) := by
  intro had_pre_record pre_was_absent
  unfold revert_user_action
  constructor
  · intro h
    split at h
    · next c1 =>
      split at h
      · next c2 => exact absurd h (by simp)
      · next c2 => exact ⟨c1, by simp at c2; exact c2⟩
    · next c1 => exact absurd h (by simp)
  · rintro ⟨hd, hp⟩
    rw [if_pos hd, if_neg (by simp [hp])]

/-- AS-IS dente: a missing preimage record is NOT "absent" — the as-is
    blind-deletes a user key the peer never prepared (F34). -/
theorem revert_user_action_as_is_dente :
    revert_user_action_as_is false false
      = ok RevertUserAction.RestoreAbsent := by
  unfold revert_user_action_as_is
  rfl

/-- Catalog entry: commit of an aborted txn reverts — forall over the
    status (F47: the abort fence always wins, never materialises). -/
theorem txn_commit_action_reverts_iff_abort :
    ∀ (status_is_abort : Bool),
      (txn_commit_action status_is_abort
          = ok TxnCommitAction.Revert)
        ↔ (status_is_abort = true) := by
  intro status_is_abort
  unfold txn_commit_action
  constructor
  · intro h
    split at h
    · next c1 => exact c1
    · next c1 => exact absurd h (by simp)
  · intro hd
    rw [if_pos hd]

/-- Catalog entry: revert may drop the txn status key only when every
    pair is gone AND the status was not abort (F47: the abort fence
    survives a later TxnCommit replay). -/
theorem revert_clears_status_clears_iff_pairs_gone_and_live :
    ∀ (status_is_abort : Bool) (pairs_empty : Bool),
      (revert_clears_status status_is_abort pairs_empty = ok true)
        ↔ (pairs_empty = true ∧ status_is_abort = false) := by
  intro status_is_abort pairs_empty
  unfold revert_clears_status
  constructor
  · intro h
    split at h
    · next c1 =>
      simp at h
      exact ⟨c1, h⟩
    · next c1 => exact absurd h (by simp)
  · rintro ⟨hp, ha⟩
    rw [if_pos hp, ha]
    rfl

/-- RFC-0227 P1.4 T1 close over recover: leftover abort never
    materializes; committed leftover stays. -/
theorem t1_recover :
    ∀ (committed : Bool),
      txn_recover_materializes committed = ok committed := by
  intro committed
  unfold txn_recover_materializes leftover_fate txn_commit_action
  cases committed <;> simp

theorem txn_recover_materializes_as_is_dente :
    txn_recover_materializes_as_is false = ok true := by
  unfold txn_recover_materializes_as_is
  rfl
