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
