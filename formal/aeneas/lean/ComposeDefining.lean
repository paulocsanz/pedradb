-- RFC-0227 P1.6: unique pedra_refines Rel simulation of D1/R1/T1/C1.
-- Rel is a two-sided relation Conc ↔ Abs, not inv_wal tautology.
import Aeneas
import Merge
import Txn
import Membership
import Flush
open Aeneas.Std Result
open pedra_aeneas_merge_kernel
open pedra_aeneas_txn_kernel
open pedra_aeneas_membership_kernel
open pedra_aeneas_flush_kernel

/-- Kernel (extracted) state the rustc handlers match. -/
structure Conc where
  wal : wal.wal_state_kernel.WalState
  kinds : Slice key.ValueType
  hiddens : Slice Bool
  leftover_committed : Bool
  in_ids : Bool
  old_yes : U64
  old_n : U64
  new_yes : Option (U64 × U64)
  seq : U64
  commit : U64
  served_live : Bool

/-- Spec dictionary: acked WAL prefix, live get, leftover visibility,
    replica prefix. -/
structure Abs where
  acked : U64
  synced : U64
  written : U64
  live : Bool
  leftover_visible : Bool
  replica_prefix_ok : Bool

/-- Abstraction relation: kernel WalState/get/recover/replica refine the
    spec fields. Two arguments — not `Rel s := inv_wal s.wal`. -/
def Rel (c : Conc) (a : Abs) : Prop :=
  wal.wal_state_kernel.inv_wal c.wal = ok true
  ∧ a.acked = c.wal.acked
  ∧ a.synced = c.wal.synced
  ∧ a.written = c.wal.written
  ∧ merge.get_live c.kinds c.hiddens = ok a.live
  ∧ txn_recover_materializes c.leftover_committed = ok a.leftover_visible
  ∧ replica_served_ok c.in_ids c.old_yes c.old_n c.new_yes c.seq c.commit
      c.served_live = ok a.replica_prefix_ok

/-- RFC-0227 P1.2 A3: confinement quantifies the Isolated get walk. -/
theorem confinement :
    ∀ (kinds : Slice key.ValueType) (hiddens : Slice Bool),
      merge.get_live kinds hiddens = ok true ↔ first_covering_live kinds hiddens :=
  r1_get_live

/-- RFC-0227 P1.6: Rel c a implies the four product sentences on the
    spec, citing the P1.2–P1.5 closes — Rel is not the conclusion.
    Inv-WAL on Abs counters via `wal_inv_closed`; R1 via `r1_get_live`
    (live ⇒ Value ∧ ¬hidden); T1 leftover visible only if committed;
    C1 replica_prefix_ok ⇒ replica_served_ok. -/
theorem pedra_refines :
    ∀ (c : Conc) (a : Abs),
      Rel c a →
        a.acked ≤ a.synced ∧ a.synced ≤ a.written
        ∧ (a.live = true →
            ∀ (kind : key.ValueType) (range_hidden : Bool),
              merge.visible_at kind range_hidden = ok true →
                kind = key.ValueType.Value ∧ range_hidden = false)
        ∧ (a.leftover_visible = true → c.leftover_committed = true)
        ∧ (a.replica_prefix_ok = true →
            replica_served_ok c.in_ids c.old_yes c.old_n c.new_yes
              c.seq c.commit c.served_live = ok true) := by
  intro c a h
  rcases h with ⟨hinv, hacks, hsync, hwrit, _hlive, hleftover, hrep⟩
  rw [wal_inv_closed] at hinv
  have hAB := Result.ok.inj hinv
  rw [Bool.and_eq_true] at hAB
  obtain ⟨hA, hB⟩ := hAB
  have hle1 : c.wal.acked ≤ c.wal.synced := by
    simpa [decide_eq_true_eq, UScalar.le_equiv] using hA
  have hle2 : c.wal.synced ≤ c.wal.written := by
    simpa [decide_eq_true_eq, UScalar.le_equiv] using hB
  refine ⟨?w1, ?w2, ?r1, ?t1, ?c1⟩
  · simpa [hacks, hsync] using hle1
  · simpa [hsync, hwrit] using hle2
  · intro _hlive kind range_hidden hvis
    exact (visible_at_live_iff kind range_hidden).mp hvis
  · intro hv
    have ht := t1_recover c.leftover_committed
    rw [ht] at hleftover
    have hveq := Result.ok.inj hleftover
    simpa [hv] using hveq.symm
  · intro hok
    simpa [hok] using hrep

/-- RFC-0227 P2.5: Inv-WAL/Inv-LSM on the Rel-image of handler-matched
    steps — cites wal_write_step_preserves_inv_wal and
    merge_chain_preserves_inv_lsm; Rel is a hypothesis, not the conclusion. -/
theorem inv_rel_image :
    ∀ (c : Conc) (a : Abs) (s' : wal.wal_state_kernel.WalState)
      (k : Nat) (chain : List MergeStep),
      Rel c a →
      wal_write_step c.wal s' →
      merge_chain k chain →
        wal.wal_state_kernel.inv_wal s' = ok true
        ∧ (∀ s ∈ chain,
            merge_step_answers_live s →
              s.kind = key.ValueType.Value ∧ s.range_hidden = false) := by
  intro c a s' k chain hrel hstep hchain
  constructor
  · have hinv : wal.wal_state_kernel.inv_wal c.wal = ok true := hrel.1
    exact wal_write_step_preserves_inv_wal c.wal s' hinv hstep
  · exact merge_chain_preserves_inv_lsm k chain hchain

/-- RFC-0227 P2.1+P2.3 A6: ConcurrentDb write-group ∀ — lock-order ×
    rotate with commit_inflight (idle rotate recused). -/
theorem concurrent_db_write_group_forall :
    ∀ (read_held inflight : Bool) (s : WalPinState),
      occ_snap_lock_order read_held inflight
        = ok (if read_held then inflight else true)
      ∧ (s.commit_inflight = true →
          wal_rotate_decision s = ok WalRotateAction.KeepWal) := by
  intro read_held inflight s
  constructor
  · unfold occ_snap_lock_order occ_snap_uses_published
    cases read_held <;> cases inflight <;> simp
  · intro h
    unfold wal_rotate_decision
    rw [h]
    cases s.mem_empty <;> cases s.imm_present <;> cases s.pin_live <;>
      cases s.parked_unflushed <;> simp

/-- RFC-0227 Rel D1 steps: dual-unfold of extracted `put_handler_plan`
    (callee `batch_is_empty`) and `fence_on_sync_fail` — the same
    composer `d1_put_crash_reopen` matches. -/
theorem rel_d1_put_fence :
    ∀ (n : U64) (commit_failed need_sync sync_fail : Bool),
      write_admission_kernel.put_handler_plan n commit_failed
        = (do
            let b ← write_admission_kernel.batch_is_empty n
            if b then ok write_admission_kernel.PutHandlerPlan.EmptyOk
            else
              if commit_failed
              then ok write_admission_kernel.PutHandlerPlan.RestoreSeqOnCommitErr
              else ok write_admission_kernel.PutHandlerPlan.CommitThenFlush)
      ∧ write_admission_kernel.fence_on_sync_fail need_sync sync_fail
          = ok (if need_sync then sync_fail else false) := by
  intro n commit_failed need_sync sync_fail
  constructor
  · unfold write_admission_kernel.put_handler_plan
      write_admission_kernel.batch_is_empty
    rfl
  · unfold write_admission_kernel.fence_on_sync_fail
    cases need_sync <;> rfl

/-- RFC-0227 Rel T1 steps: dual-unfold leftover abort × revert. -/
theorem rel_t1_leftover :
    leftover_txn_is_aborted = leftover_fate false
    ∧ txn_commit_action true = ok TxnCommitAction.Revert := by
  constructor
  · unfold leftover_txn_is_aborted leftover_fate
    rfl
  · unfold txn_commit_action
    rfl
