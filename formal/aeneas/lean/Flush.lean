-- Theorems over the Aeneas extract of production flush_kernel.rs
-- (RFC-0174 P1.3). Fail-closed: this file must not contain a hole.
import Aeneas
import FlushKernel
open Aeneas.Std Result
open pedra_aeneas_flush_kernel

/-- Empty idle pipeline rotates only (no SST write). -/
theorem flush_plan_empty_rotates_only :
    flush_plan true false = ok FlushPlan.RotateOnly := by
  unfold flush_plan
  rfl

/-- Non-empty mem writes an SST before any rotate (tail never dropped). -/
theorem flush_plan_nonempty_writes_sst :
    flush_plan false false = ok FlushPlan.WriteSstThenRotate := by
  unfold flush_plan
  rfl

/-- Pending imm finishes first (single-flight). -/
theorem flush_plan_imm_finishes_first :
    flush_plan false true = ok FlushPlan.FinishImmThenFlush := by
  unfold flush_plan
  rfl

/-- AS-IS dente: lose-tail always rotates. -/
theorem flush_plan_as_is_lose_tail_dente :
    flush_plan_as_is_lose_tail false false = ok FlushPlan.RotateOnly := by
  unfold flush_plan_as_is_lose_tail
  rfl

/-- Live flush read pin keeps the WAL. -/
theorem wal_rotate_pin_live_keeps :
    wal_rotate_decision
      { mem_empty := true, imm_present := false, pin_live := true,
        parked_unflushed := false, commit_inflight := false }
      = ok WalRotateAction.KeepWal := by
  unfold wal_rotate_decision
  rfl

/-- AS-IS dente: ignore-pin truncates while the pin is live. -/
theorem wal_rotate_as_is_ignores_pin :
    wal_rotate_decision_as_is_ignore_pin
      { mem_empty := true, imm_present := false, pin_live := true,
        parked_unflushed := false, commit_inflight := false }
      = ok WalRotateAction.RotateWal := by
  unfold wal_rotate_decision_as_is_ignore_pin
  rfl

/-- ConcurrentDb lock-order: a commit in the off-lock fd window keeps the WAL
    (flush must not truncate bytes the writer still owns). -/
theorem wal_rotate_commit_inflight_keeps :
    wal_rotate_decision
      { mem_empty := true, imm_present := false, pin_live := false,
        parked_unflushed := false, commit_inflight := true }
      = ok WalRotateAction.KeepWal := by
  unfold wal_rotate_decision
  rfl

/-- Other branch of the same caller: idle pipeline with no inflight rotates. -/
theorem wal_rotate_idle_rotates :
    wal_rotate_decision
      { mem_empty := true, imm_present := false, pin_live := false,
        parked_unflushed := false, commit_inflight := false }
      = ok WalRotateAction.RotateWal := by
  unfold wal_rotate_decision
  rfl

/-- Write-lock client protocol: inflight still keeps even if the as-is
    mutant ignores the flush pin (commit_inflight is not the pin hole). -/
theorem wal_rotate_inflight_keeps_on_as_is_pin :
    wal_rotate_decision_as_is_ignore_pin
      { mem_empty := true, imm_present := false, pin_live := false,
        parked_unflushed := false, commit_inflight := true }
      = ok WalRotateAction.KeepWal := by
  unfold wal_rotate_decision_as_is_ignore_pin
  rfl
