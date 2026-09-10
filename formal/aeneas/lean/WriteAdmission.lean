-- Theorems over the Aeneas extract of production write_admission_kernel.rs
-- (RFC-0170 P2.1). Fail-closed: this file must not contain a hole.
import Aeneas
import WriteAdmissionKernel
open Aeneas.Std Result
open pedra_aeneas_write_admission_kernel

/-- Extracted idle gate is true iff every stall knob is off. -/
theorem write_admission_idle_matches_spec :
    write_admission_idle false false false = ok true := by
  unfold write_admission_idle
  rfl

/-- A live mem-stall knob refuses the idle path. -/
theorem write_admission_idle_mem_stall_refuses :
    write_admission_idle true false false = ok false := by
  unfold write_admission_idle
  rfl

/-- AS-IS dente: stall knobs are ignored (always idle). -/
theorem write_admission_idle_as_is_dente :
    write_admission_idle_as_is true true true = ok true := by
  unfold write_admission_idle_as_is
  rfl

/-- Hard admit: mem over an armed limit is StallMem. -/
theorem write_admit_mem_over_stalls :
    write_admit 100#u64 true 50#u64 0#u64 false 0#u64
      = ok WriteAdmit.StallMem := by
  unfold write_admit
  have h : (100#u64 ≥ 50#u64) = true := by native_decide
  simp [h]

/-- AS-IS dente: mem over still admits. -/
theorem write_admit_as_is_dente :
    write_admit_as_is 100#u64 true 50#u64 8#u64 true 4#u64
      = ok WriteAdmit.Ok := by
  unfold write_admit_as_is
  rfl

/-- Put-Ok: client WriteOptions.sync=true requires a WAL barrier. -/
theorem wal_sync_required_client_true :
    wal_sync_required true true false = ok true := by
  unfold wal_sync_required
  rfl

/-- Open-options sync requires a directory fsync after rename/create. -/
theorem dir_sync_required_when_sync :
    dir_sync_required true = ok true := by
  unfold dir_sync_required
  rfl

/-- Put-Ok script: required sync that succeeded is Sync before Apply/Ok. -/
theorem wal_commit_plan_need_sync_ok :
    wal_commit_plan true false = ok WalCommitPlan.AppendSyncApplyOk := by
  unfold wal_commit_plan
  rfl

/-- Required sync failed ⇒ Fence (no Apply/Ok). Unfolds the plan rustc
    links **and** `fence_on_sync_fail` (the callee the plan now calls). -/
theorem wal_commit_plan_fence_via_fence_on_sync_fail :
    fence_on_sync_fail true true = ok true ∧
      wal_commit_plan true true = ok WalCommitPlan.AppendSyncFence := by
  constructor
  · unfold fence_on_sync_fail; rfl
  · unfold wal_commit_plan
    unfold fence_on_sync_fail
    rfl

/-- AS-IS dente: Apply/Ok even after a failed required sync. -/
theorem wal_commit_plan_as_is_dente :
    wal_commit_plan_as_is true true = ok WalCommitPlan.AppendSyncApplyOk := by
  unfold wal_commit_plan_as_is
  rfl

/-- RFC-0191 P1.2 D1-script: the whole Bool×Bool space of the plan rustc
links (`commit_ops_with` matches it). Required sync that succeeded is
Sync before Apply/Ok; required sync that failed is Fence (never
Apply/Ok); no required sync is Apply/Ok without Sync. Concrete
`wal_commit_plan true false` does **not** pay this — the binder covers
the space. -/
theorem d1_wal_commit_plan :
    ∀ (need_sync sync_fail : Bool),
      wal_commit_plan need_sync sync_fail
        = ok (if need_sync then
                (if sync_fail then WalCommitPlan.AppendSyncFence
                 else WalCommitPlan.AppendSyncApplyOk)
              else WalCommitPlan.AppendApplyOk) := by
  intro need_sync sync_fail
  unfold wal_commit_plan fence_on_sync_fail
  cases need_sync <;> cases sync_fail <;> rfl

/-- `put_if_absent`: no live key ⇒ put. -/
theorem cas_absent_put_empty_puts :
    cas_absent_put false = ok true := by
  unfold cas_absent_put
  rfl

/-- Live key ⇒ do not put (CasMismatch). -/
theorem cas_absent_put_live_refuses :
    cas_absent_put true = ok false := by
  unfold cas_absent_put
  rfl

/-- AS-IS dente: live key still puts. -/
theorem cas_absent_put_as_is_dente :
    cas_absent_put_as_is true = ok true := by
  unfold cas_absent_put_as_is
  rfl

/-- `put_if_eq`: live == expected ⇒ put. -/
theorem cas_eq_put_match_puts :
    cas_eq_put true = ok true := by
  unfold cas_eq_put
  rfl

/-- Mismatch ⇒ do not put (CasMismatch). -/
theorem cas_eq_put_mismatch_refuses :
    cas_eq_put false = ok false := by
  unfold cas_eq_put
  rfl

/-- AS-IS dente: mismatch still puts. -/
theorem cas_eq_put_as_is_dente :
    cas_eq_put_as_is false = ok true := by
  unfold cas_eq_put_as_is
  rfl
