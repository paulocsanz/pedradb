-- D1 spine: put script × WAL collector (RFC-0157 plan the handler matches).
-- Andreakis 2026 (arXiv:2608.00501): a store-side observation is not
-- recovery evidence; a complete Record is KeepRecord (the log is the
-- sink of the put). Dual-unfold of put_handler_plan AND recover_collect_act.
import Aeneas
import WriteAdmission
import WalRecover
open Aeneas.Std Result
open pedra_aeneas_write_admission_kernel
open pedra_aeneas_wal_recover_kernel

/-- Empty batch skips WAL; non-empty commit-ok is CommitThenFlush; a
    framed Record is KeepRecord on every prefix. AS-IS put always flushes.
    Unfolds the plan `apply_batch_with` matches and the WAL collector. -/
theorem put_handler_recover_spine :
    ∀ (n : U64) (commit_failed : Bool)
      (prefix_n : U64) (can_skip : Bool) (skips : U64) (in_resync : Bool),
      put_handler_plan n commit_failed
        = (do
            let b ← batch_is_empty n
            if b then ok PutHandlerPlan.EmptyOk
            else if commit_failed then ok PutHandlerPlan.RestoreSeqOnCommitErr
            else ok PutHandlerPlan.CommitThenFlush)
      ∧ recover_kernel.recover_collect_act recover_kernel.RecoverKind.Record
          prefix_n can_skip skips in_resync
        = ok recover_kernel.RecoverAct.KeepRecord
      ∧ put_handler_plan_as_is n commit_failed
        = ok PutHandlerPlan.CommitThenFlush := by
  intro n commit_failed prefix_n can_skip skips in_resync
  refine ⟨?put, ?rec, ?asis⟩
  · unfold put_handler_plan
    rfl
  · unfold recover_kernel.recover_collect_act
    rfl
  · unfold put_handler_plan_as_is
    rfl
