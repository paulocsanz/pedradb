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

/-- Lock-order client: inflight ⇒ OCC uses published seq, and rotate keeps WAL. -/
theorem occ_snap_published_and_rotate_keeps :
    occ_snap_uses_published true = ok true ∧
      wal_rotate_decision
          { mem_empty := true, imm_present := false, pin_live := false,
            parked_unflushed := false, commit_inflight := true }
        = ok WalRotateAction.KeepWal := by
  constructor
  · unfold occ_snap_uses_published; rfl
  · unfold wal_rotate_decision; rfl

/-- AS-IS dente: last_seq while inflight. -/
theorem occ_snap_uses_published_as_is_dente :
    occ_snap_uses_published_as_is true = ok false := by
  unfold occ_snap_uses_published_as_is
  rfl

/-- Write-lock client: no read lock ⇒ published snap. Unfolds the plan
    rustc links (`occ_snap_lock_order`) and the inflight callee. -/
theorem occ_snap_lock_order_write_held :
    occ_snap_lock_order false false = ok true ∧
      occ_snap_uses_published false = ok false := by
  constructor
  · unfold occ_snap_lock_order
    unfold occ_snap_uses_published
    rfl
  · unfold occ_snap_uses_published; rfl

/-- Other branch: read lock held and idle pipeline uses last_seq. -/
theorem occ_snap_lock_order_idle_read :
    occ_snap_lock_order true false = ok false ∧
      occ_snap_uses_published false = ok false := by
  constructor
  · unfold occ_snap_lock_order
    unfold occ_snap_uses_published
    rfl
  · unfold occ_snap_uses_published; rfl

/-- Read lock held but a commit owns the WAL ⇒ published. -/
theorem occ_snap_lock_order_inflight_read :
    occ_snap_lock_order true true = ok true ∧
      occ_snap_uses_published true = ok true := by
  constructor
  · unfold occ_snap_lock_order
    unfold occ_snap_uses_published
    rfl
  · unfold occ_snap_uses_published; rfl

/-- AS-IS dente: last_seq even when the write lock is held. -/
theorem occ_snap_lock_order_as_is_dente :
    occ_snap_lock_order_as_is false true = ok false := by
  unfold occ_snap_lock_order_as_is
  rfl

/-- `try_rotate_wal`: idle pipeline may rotate **and** empty segment skips.
    Unfolds the plan rustc links (`wal_rotate_decision`) and the empty-skip
    callee (`wal_segment_is_empty`). -/
theorem try_rotate_idle_empty_segment_skips :
    wal_rotate_decision
        { mem_empty := true, imm_present := false, pin_live := false,
          parked_unflushed := false, commit_inflight := false }
      = ok WalRotateAction.RotateWal ∧
      wal_segment_is_empty 0#u64 = ok true := by
  constructor
  · unfold wal_rotate_decision; rfl
  · unfold wal_segment_is_empty; rfl

/-- Other branch: a nonempty segment is not skipped. -/
theorem wal_segment_is_empty_nonzero :
    wal_segment_is_empty 1#u64 = ok false := by
  unfold wal_segment_is_empty
  rfl

/-- AS-IS dente: never skip empty (idle poll would rewrite MANIFEST). -/
theorem wal_segment_is_empty_as_is_dente :
    wal_segment_is_empty_as_is 0#u64 = ok false := by
  unfold wal_segment_is_empty_as_is
  rfl
