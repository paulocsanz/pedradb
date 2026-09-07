-- Cross-lib: ConcurrentDb write-group callees (group_commit + flush rotate).
import Aeneas
import GroupCommitKernel
import FlushKernel
open Aeneas.Std Result

/-- Off-lock protocol: failed WAL I/O does not publish, and an inflight
    commit keeps the WAL (flush must not truncate the writer's bytes). -/
theorem concurrent_publish_and_inflight_keep_wal :
    pedra_aeneas_group_commit_kernel.may_publish_group false = ok false
      ∧ pedra_aeneas_flush_kernel.wal_rotate_decision
          { mem_empty := true, imm_present := false, pin_live := false,
            parked_unflushed := false, commit_inflight := true }
        = ok pedra_aeneas_flush_kernel.WalRotateAction.KeepWal := by
  constructor
  · unfold pedra_aeneas_group_commit_kernel.may_publish_group
    rfl
  · unfold pedra_aeneas_flush_kernel.wal_rotate_decision
    rfl

/-- Other branch: WAL I/O Ok may publish, and idle pipeline rotates. -/
theorem concurrent_publish_ok_and_idle_rotates :
    pedra_aeneas_group_commit_kernel.may_publish_group true = ok true
      ∧ pedra_aeneas_flush_kernel.wal_rotate_decision
          { mem_empty := true, imm_present := false, pin_live := false,
            parked_unflushed := false, commit_inflight := false }
        = ok pedra_aeneas_flush_kernel.WalRotateAction.RotateWal := by
  constructor
  · unfold pedra_aeneas_group_commit_kernel.may_publish_group
    rfl
  · unfold pedra_aeneas_flush_kernel.wal_rotate_decision
    rfl

/-- AS-IS dente: publish after failed WAL, but inflight still keeps (the
    pin-hole mutant does not drop commit_inflight). -/
theorem concurrent_as_is_publish_lie_inflight_still_keeps :
    pedra_aeneas_group_commit_kernel.may_publish_group_as_is false = ok true
      ∧ pedra_aeneas_flush_kernel.wal_rotate_decision_as_is_ignore_pin
          { mem_empty := true, imm_present := false, pin_live := false,
            parked_unflushed := false, commit_inflight := true }
        = ok pedra_aeneas_flush_kernel.WalRotateAction.KeepWal := by
  constructor
  · unfold pedra_aeneas_group_commit_kernel.may_publish_group_as_is
    rfl
  · unfold pedra_aeneas_flush_kernel.wal_rotate_decision_as_is_ignore_pin
    rfl
