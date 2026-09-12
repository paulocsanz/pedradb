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

/-- RFC-0213 P1.1 (storage cadence, atom `catalog:flush_publish`):
    the manifest may publish EXACTLY when the SST is durable — fate
    forall over the extracted body (RFC-0170 P2.4); the AS-IS mutant
    publishes unsynced SSTs (the lie the DST plant
    `may_publish_manifest_on_live_unsynced_sst_is_not_ok` refutes). -/
theorem flush_publish_fate_iff :
    ∀ (sst_durable v : Bool),
      (may_publish_manifest sst_durable = ok v) ↔ v = sst_durable := by
  intro sst_durable v
  unfold may_publish_manifest
  cases sst_durable <;> cases v <;> simp

/-- RFC-0213 P1.1 (storage cadence, atom `catalog:auto_flush_due`):
    the auto-flush is due EXACTLY when the axis is armed and the
    bytes reached the limit — fate forall over the extracted body
    (RFC-0170 P2.4); the AS-IS mutant never flushes on its own (the
    lie the DST plant `auto_flush_due_on_live_over_limit_is_not_ok`
    refutes). -/
theorem auto_flush_due_fate_iff :
    ∀ (mem_bytes : U64) (armed : Bool) (limit : U64) (v : Bool),
      (auto_flush_due mem_bytes armed limit = ok v) ↔
        ((v = decide (mem_bytes >= limit) ∧ armed = true)
          ∨ (v = false ∧ armed = false)) := by
  intro mem_bytes armed limit v
  unfold auto_flush_due
  cases armed <;> simp <;> exact eq_comm

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

/-- RFC-0198 P2.2 (first cap-descent if): the OCC snapshot reads the
    published seq exactly when a commit is inflight — the computation
    rule of the do-block body (a pure lift; the caller's off-lock window
    decides the snapshot's visibility base). -/
theorem occ_snap_uses_published_ok_iff_inflight :
    ∀ (commit_inflight v : Bool),
      (occ_snap_uses_published commit_inflight = ok v) ↔ (commit_inflight = v) := by
  intro commit_inflight v
  unfold occ_snap_uses_published
  constructor
  · intro h
    injection h with _
  · intro h
    rw [h]

/-- Any ok-valued Result bind forces the bound term to be ok. -/
private theorem bind_ok_inv {α β} (x : Result α) (f : α → Result β) (v : β)
    (h : Aeneas.Std.bind x f = ok v) : ∃ a, x = ok a ∧ f a = ok v := by
  cases x with
  | ok a => exact ⟨a, rfl, h⟩
  | fail e => exact absurd h (by simp)
  | div => exact absurd h (by simp)

/-- An ok chain reassembles into an ok bind. -/
private theorem bind_intro {α β} {x : Result α} {f : α → Result β} {v : β}
    (a : α) (hx : x = ok a) (h : f a = ok v) : Aeneas.Std.bind x f = ok v := by
  rw [hx]
  exact h

/-- RFC-0200 P0.2 (fourth registered glue close): the caller step of
    `try_rotate_wal` (db.rs) composes the plan `wal_rotate_decision`
    with the callee `wal_segment_is_empty` in exactly this order —
    decision first (KeepWal skips), inflight recheck under the WAL
    mutex skips, and an EMPTY segment skips (an idle poll must never
    rewrite MANIFEST). The step fires `rotate_wal_now` EXACTLY when
    the decision is RotateWal, the recheck is idle and the segment
    HAS data. -/
theorem try_rotate_step_rotates_iff_pins_clear_segment_live :
    ∀ (s : WalPinState) (recheck_inflight : Bool) (pos : Aeneas.Std.U64),
      (Aeneas.Std.bind (wal_rotate_decision s)
        (fun a =>
          match a with
          | WalRotateAction.RotateWal =>
              if recheck_inflight = true then ok false
              else Aeneas.Std.bind (wal_segment_is_empty pos)
                (fun e => ok (!e))
          | WalRotateAction.KeepWal => ok false)) = ok true ↔
      (wal_rotate_decision s = ok WalRotateAction.RotateWal
        ∧ recheck_inflight = false
        ∧ wal_segment_is_empty pos = ok false) := by
  intro s recheck_inflight pos
  constructor
  · intro hval
    obtain ⟨a, hw, hm⟩ := bind_ok_inv _ _ _ hval
    split at hm
    · split at hm
      · next _ =>
          injection hm with hm'
          simp at hm'
      · next hr =>
          obtain ⟨e, he, hfin⟩ := bind_ok_inv _ _ _ hm
          injection hfin with hnot
          have hef : e = false := by
            cases e with
            | true => exact absurd hnot (by simp)
            | false => rfl
          rw [hef] at he
          refine ⟨hw, ?_, he⟩
          cases recheck_inflight with
          | true => exact absurd rfl hr
          | false => rfl
    · injection hm with hm'
      simp at hm'
  · rintro ⟨hw, hr, he⟩
    refine bind_intro _ hw ?_
    show (if recheck_inflight = true then ok false
        else Aeneas.Std.bind (wal_segment_is_empty pos)
          (fun e => ok (!e))) = ok true
    rw [hr, if_neg (by simp)]
    exact bind_intro _ he rfl
