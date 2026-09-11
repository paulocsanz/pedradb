-- RFC-0199 (P1.1): the Work.io algebra — named IO primitives as
-- countable constructors — and the confirmed-write-path barrier count.
-- The algebra counts CONSTRUCTORS (how many pwrite/fdatasync/pread/
-- fadvise a path executes); nanoseconds stay dated measured anchors in
-- the rust kernels (write_cycle_kernel), never theorems (0187). The
-- barrier primitive itself is the extracted posix `fdatasync_rc_ok`
-- (rc = 0 is the only ok); the constructor Work.fdatasync counts
-- exactly one such call. The registered theorem: a wal commit plan
-- that APPLIES OK (a committed group) pays at most one fdatasync —
-- exactly one when the group asked for sync, zero otherwise; the fence
-- outcome is a refusal, not a commit.
import Aeneas
import WriteAdmissionKernel
import PosixKernel
open Aeneas Aeneas.Std Result ControlFlow
open pedra_aeneas_write_admission_kernel pedra_aeneas_posix_kernel

/-! ## The Work.io algebra (RFC-0199 P1.1) -/

/-- IO work: a countable model of the named primitives the confirmed
paths execute. `bytes` payloads keep the model composable with size
measures; only the constructor multiset is formal. -/
inductive Work where
  | ret : Work
  | pwrite (bytes : Nat) : Work
  | fdatasync : Work
  | pread (bytes : Nat) : Work
  | fadvise : Work
  | seq (w₁ w₂ : Work) : Work

/-- How many `fdatasync` barriers a work expression executes. -/
def Work.fdatasync_count : Work → Nat
  | Work.ret => 0
  | Work.pwrite _ => 0
  | Work.fdatasync => 1
  | Work.pread _ => 0
  | Work.fadvise => 0
  | Work.seq w₁ w₂ => w₁.fdatasync_count + w₂.fdatasync_count

/-- How many `pwrite`s a work expression executes. -/
def Work.pwrite_count : Work → Nat
  | Work.ret => 0
  | Work.pwrite _ => 1
  | Work.fdatasync => 0
  | Work.pread _ => 0
  | Work.fadvise => 0
  | Work.seq w₁ w₂ => w₁.pwrite_count + w₂.pwrite_count

/-! ## The barrier primitive (posix extract) -/

/-- The extracted barrier admits exactly rc = 0 — every other return
code is a failed barrier (the counted constructor is this call). -/
theorem fdatasync_rc_ok_iff_zero : ∀ (rc : Std.I32),
    fdatasync_rc_ok rc = ok true ↔ rc = 0#i32 := by
  intro rc
  simp [fdatasync_rc_ok]

/-! ## The confirmed write path -/

/-- A plan applies ok (the group COMMITS) — the fence outcome is a
refusal, never a commit. -/
def wal_commit_applies_ok : WalCommitPlan → Bool
  | WalCommitPlan.AppendApplyOk => true
  | WalCommitPlan.AppendSyncApplyOk => true
  | WalCommitPlan.AppendSyncFence => false

/-- Work interpretation of a plan: AppendApplyOk commits without a
barrier; AppendSyncApplyOk pays exactly one barrier before Ok; the
fence paid its (failed) barrier and refused. -/
def wal_commit_work : WalCommitPlan → Work
  | WalCommitPlan.AppendApplyOk => Work.ret
  | WalCommitPlan.AppendSyncApplyOk => Work.fdatasync
  | WalCommitPlan.AppendSyncFence => Work.fdatasync

/-- ok chains: a bind equal to an ok value forces the bound operation
to have returned ok (Cf.lean's `bind_ok_inv`, restated for this
module). -/
private theorem bind_ok_inv {α β} (x : Result α) (f : α → Result β) (v : β)
    (h : Aeneas.Std.bind x f = ok v) : ∃ a, x = ok a ∧ f a = ok v := by
  cases x with
  | ok a => exact ⟨a, rfl, h⟩
  | fail e => simp at h
  | div => simp at h

/-- Sharp count: a COMMITTED group pays exactly one barrier when it
asked for sync, zero when it did not — the barrier is per group, not
per write in the batch. -/
theorem wal_commit_plan_committed_sync_count :
    ∀ (need_sync sync_failed : Bool) (p : WalCommitPlan),
      wal_commit_plan need_sync sync_failed = ok p →
      wal_commit_applies_ok p = true →
      (wal_commit_work p).fdatasync_count = if need_sync = true then 1 else 0 := by
  intro need_sync sync_failed p hval hap
  unfold wal_commit_plan at hval
  obtain ⟨b, _, hval⟩ := bind_ok_inv _ _ _ hval
  split at hval
  · next hb =>
      injection hval with hv
      subst hv
      simp [wal_commit_applies_ok] at hap
  · next hb =>
      split at hval
      · next hns =>
          injection hval with hv
          subst hv
          simp [wal_commit_work, Work.fdatasync_count, hns]
      · next hns =>
          injection hval with hv
          subst hv
          simp [wal_commit_work, Work.fdatasync_count, hns]

/-- RFC-0199 count (P1.1): a committed group executes at most one
`fdatasync` — group commit amortizes the barrier over the whole batch;
no single committed write ever pays two. (The fence outcome also pays
at most one: the failed barrier plus refusal.) -/
theorem wal_commit_plan_at_most_one_fdatasync :
    ∀ (need_sync sync_failed : Bool) (p : WalCommitPlan),
      wal_commit_plan need_sync sync_failed = ok p →
      (wal_commit_work p).fdatasync_count ≤ 1 := by
  intro need_sync sync_failed p hval
  unfold wal_commit_plan at hval
  obtain ⟨b, _, hval⟩ := bind_ok_inv _ _ _ hval
  split at hval
  · injection hval with hv
    subst hv
    simp only [wal_commit_work]
    decide
  · split at hval
    · injection hval with hv
      subst hv
      simp only [wal_commit_work]
      decide
    · injection hval with hv
      subst hv
      simp only [wal_commit_work]
      decide
