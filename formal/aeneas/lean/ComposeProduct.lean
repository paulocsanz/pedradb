-- RFC-0227 P2.2–P2.4 dual-unfold spines (caller def AND callee def).
import Aeneas
import Flush
import GroupCommit
import WriteAdmission
import Changelog
import Locktab
open Aeneas.Std Result
open pedra_aeneas_flush_kernel
open pedra_aeneas_group_commit_kernel
open pedra_aeneas_write_admission_kernel
open pedra_aeneas_changelog_kernel
open pedra_aeneas_locktab_kernel

/-- RFC-0227 P2.4 auto-flush spine: skip-all iff both under; mem flush iff
    armed and at limit; CF flush iff at limit. Dual-unfold of three defs. -/
theorem auto_flush_spine :
    ∀ (global_under cf_under armed : Bool) (mem_bytes limit : U64),
      auto_flush_gate global_under cf_under
        = ok (if global_under = true ∧ cf_under = true
              then AutoFlushGate.SkipAllNotDue
              else AutoFlushGate.ScanColumnFamilies)
      ∧ mem_auto_flush_plan mem_bytes armed limit
        = (do
            let b ← auto_flush_due mem_bytes armed limit
            if b then ok MemAutoFlushPlan.FlushMemNow
            else ok MemAutoFlushPlan.NotDueKeepMem)
      ∧ cf_flush_plan mem_bytes limit
        = (do
            let b ← auto_flush_due mem_bytes true limit
            if b then ok CfFlushPlan.FlushCfNow
            else ok CfFlushPlan.CfNotDueSkip) := by
  intro global_under cf_under armed mem_bytes limit
  refine ⟨?g, ?m, ?c⟩
  · unfold auto_flush_gate skip_auto_flush
    cases global_under <;> cases cf_under <;> simp
  · unfold mem_auto_flush_plan auto_flush_due
    rfl
  · unfold cf_flush_plan auto_flush_due
    rfl

/-- RFC-0227 P2.4 parked_pop × group_ack. -/
theorem parked_pop_group_ack_spine :
    ∀ (parked_len : U64) (wal_io_ok : Bool),
      parked_pop_plan parked_len
        = ok (if parked_len = 0#u64
              then ParkedPopPlan.NoParkedTables
              else ParkedPopPlan.PopOldestParked)
      ∧ group_ack_plan wal_io_ok
        = ok (if wal_io_ok = true
              then GroupAckPlan.AckPublishGroup
              else GroupAckPlan.FenceRefuseIoFail) := by
  intro parked_len wal_io_ok
  constructor
  · unfold parked_pop_plan batch_is_empty
    simp
    split <;> rfl
  · unfold group_ack_plan may_publish_group
    cases wal_io_ok <;> simp

/-- RFC-0227 P2.4 changelog_store × pit_resync. -/
theorem changelog_pit_resync_spine :
    ∀ (publish_ok is_resync : Bool),
      changelog_store_plan publish_ok
        = ok (if publish_ok = true
              then ChangelogStorePlan.StoreFeed
              else ChangelogStorePlan.SkipStorePublishHolds)
      ∧ pit_resync_rewrite_plan is_resync
        = ok (if is_resync = true
              then PitResyncRewritePlan.RewriteWalFromPrefix
              else PitResyncRewritePlan.KeepRecoveredPrefix) := by
  intro publish_ok is_resync
  constructor
  · unfold changelog_store_plan
    cases publish_ok <;> simp
  · unfold pit_resync_rewrite_plan pit_resync_needs_rewrite
    cases is_resync <;> simp

/-- RFC-0227 P2.3 N-way OCC of the plan validate_occ_batch matches.
    Unfolds occ_batch_plan AND group_validate (and occ_conflict). -/
theorem occ_nway_handler :
    group_validate
        (⟨[{ snap := 10#u64, touched_key_written_after := true },
           { snap := 10#u64, touched_key_written_after := true },
           { snap := 7#u64, touched_key_written_after := true }],
          by native_decide⟩)
        (10#u64) =
      ok (⟨[false, false, true], by native_decide⟩ : alloc.vec.Vec Bool)
    ∧ occ_batch_plan
        (⟨[false, false, false], by native_decide⟩)
        (⟨[{ snap := 10#u64, touched_key_written_after := true },
           { snap := 10#u64, touched_key_written_after := true },
           { snap := 7#u64, touched_key_written_after := true }],
          by native_decide⟩)
        (10#u64) =
      ok (⟨[OccMemberFate.Ok, OccMemberFate.Ok, OccMemberFate.Conflict],
            by native_decide⟩
        : alloc.vec.Vec OccMemberFate) := by
  constructor
  · unfold group_validate
    exact LawfulBEq.eq_of_beq (by native_decide)
  · unfold occ_batch_plan
    exact LawfulBEq.eq_of_beq (by native_decide)

/-- RFC-0227 P2.2 2PL: dual-unfold of `wait_for_deadlock` (the rustc
    entry) and `wait_for_deadlock_loop` (the cycle walk). The step that
    closes a wait-for cycle is `wait_for_deadlock_step_cycle_closes`
    (Locktab.lean). -/
theorem wait_for_deadlock_cycle :
    ∀ owned waiting waiter owner seen seen' k,
      std.collections.hash.set.HashSet.insert core.cmp.EqU64
          U64.Insts.CoreHashHash
          std.hash.random.RandomState.Insts.CoreHashBuildHasherDefaultHasher
          seen owner = ok (true, seen') →
      std.collections.hash.map.HashMap.get core.cmp.EqU64
          U64.Insts.CoreHashHash
          std.hash.random.RandomState.Insts.CoreHashBuildHasherDefaultHasher
          (core.borrow.Borrow.Blanket Aeneas.Std.U64) U64.Insts.CoreHashHash
          core.cmp.EqU64 waiting owner = ok (some k) →
      std.collections.hash.map.HashMap.get bytes.bytes.Bytes.Insts.CoreCmpEq
          bytes.bytes.Bytes.Insts.CoreHashHash
          std.hash.random.RandomState.Insts.CoreHashBuildHasherDefaultHasher
          (core.borrow.Borrow.Blanket bytes.bytes.Bytes)
          bytes.bytes.Bytes.Insts.CoreHashHash
          bytes.bytes.Bytes.Insts.CoreCmpEq owned k = ok (some waiter) →
      wait_for_deadlock_loop.body owned waiting waiter owner seen
        = ok (ControlFlow.done true)
      ∧ wait_for_deadlock owned waiting waiter owner = (
          do
            let seen0 ← std.collections.hash.set.HashSetTRandomStateGlobal.new U64
            wait_for_deadlock_loop owned waiting waiter owner seen0) := by
  intro owned waiting waiter owner seen seen' k hseen hwait hkey
  constructor
  · unfold wait_for_deadlock_loop.body
    rw [hseen, hwait]
    simp [hkey]
  · unfold wait_for_deadlock wait_for_deadlock_loop
    rfl

/-- leftover Env-adjacent open script: the plan `open_with_env_sourced`
    matches dual-unfolds `torn_head_is_empty_log`. Missing WAL skips;
    Truncated(0) on a tiny file is EmptyTiny; else RecoverSpan.
    AS-IS always RecoverSpan. -/
theorem open_wal_head_torn_spine :
    ∀ (wal_exists truncated_zero : Bool) (wal_len : U64),
      open_wal_head_plan wal_exists truncated_zero wal_len
        = ok (if wal_exists = false then OpenWalHeadPlan.Skip
            else if truncated_zero = true && decide (wal_len < TINY_WAL_EMPTY_MAX)
              then OpenWalHeadPlan.EmptyTiny
              else OpenWalHeadPlan.RecoverSpan)
      ∧ torn_head_is_empty_log wal_len TINY_WAL_EMPTY_MAX
          = ok (decide (wal_len < TINY_WAL_EMPTY_MAX))
      ∧ open_wal_head_plan_as_is wal_exists truncated_zero wal_len
          = ok OpenWalHeadPlan.RecoverSpan := by
  intro wal_exists truncated_zero wal_len
  refine ⟨?plan, ?head, ?asis⟩
  · unfold open_wal_head_plan torn_head_is_empty_log
    cases wal_exists with
    | false => simp
    | true =>
        cases truncated_zero with
        | false => simp
        | true =>
            simp
            split <;> rfl
  · unfold torn_head_is_empty_log
    simp
  · unfold open_wal_head_plan_as_is
    rfl
