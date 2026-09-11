-- Cross-lib: ConcurrentDb write-group callees (group_commit + flush rotate).
import Aeneas
import GroupCommitKernel
import FlushKernel
import Flush
open Aeneas.Std Result

/-! ### RFC-0205 P1.1 — the off-lock protocol as foralls over the two libs

(a) publish fate ∀ (corollary tier over the registered P0.1 close
`may_publish_group_ok_iff_wal_io_ok`, GroupCommit.lean); (b) rotate fate
∀ over the pin record — the exact keep-disjunction comes from the
extracted body, not intuition; (c) the concrete dentes below become
COROLLARIES of these foralls by instantiation. No registry row: the
compose tier spans two kernel libs and registration requires a single
catalog pair/entry (same reason as the other compose libs). -/

/-- (a) The publish side of the off-lock protocol for ALL inputs: the
    pure-lift body admits exactly the two fates, no third outcome. -/
theorem concurrent_publish_fate_forall :
    ∀ (wal_io_ok : Bool),
      pedra_aeneas_group_commit_kernel.may_publish_group wal_io_ok
        = ok wal_io_ok := by
  intro wal_io_ok
  unfold pedra_aeneas_group_commit_kernel.may_publish_group
  rfl

/-- (b) The rotate side for ALL pin records: RotateWal EXACTLY when the
    record is all-clear (empty mem, no imm, no live pin, nothing parked,
    no inflight commit) — a busy record NEVER truncates the writer's
    WAL; KeepWal is the exact complement. -/
theorem wal_rotate_decision_fate_forall :
    ∀ (s : pedra_aeneas_flush_kernel.WalPinState),
      (pedra_aeneas_flush_kernel.wal_rotate_decision s
          = ok pedra_aeneas_flush_kernel.WalRotateAction.RotateWal
        ↔ (s.mem_empty = true ∧ s.imm_present = false ∧ s.pin_live = false
            ∧ s.parked_unflushed = false ∧ s.commit_inflight = false)) ∧
      (pedra_aeneas_flush_kernel.wal_rotate_decision s
          = ok pedra_aeneas_flush_kernel.WalRotateAction.KeepWal
        ↔ ¬(s.mem_empty = true ∧ s.imm_present = false ∧ s.pin_live = false
            ∧ s.parked_unflushed = false ∧ s.commit_inflight = false)) := by
  intro s
  simp only [pedra_aeneas_flush_kernel.wal_rotate_decision]
  cases s.mem_empty <;> cases s.imm_present <;> cases s.pin_live <;>
    cases s.parked_unflushed <;> cases s.commit_inflight <;>
    simp

/-- AS-IS twin of the rotate forall: the pin-hole mutant drops
    `pin_live` from the clear-record disjunction — a LIVE PIN ALONE no
    longer keeps the WAL (the lie the DST twins pin). -/
theorem wal_rotate_decision_as_is_ignore_pin_fate_forall :
    ∀ (s : pedra_aeneas_flush_kernel.WalPinState),
      (pedra_aeneas_flush_kernel.wal_rotate_decision_as_is_ignore_pin s
          = ok pedra_aeneas_flush_kernel.WalRotateAction.RotateWal
        ↔ (s.mem_empty = true ∧ s.imm_present = false
            ∧ s.parked_unflushed = false ∧ s.commit_inflight = false)) ∧
      (pedra_aeneas_flush_kernel.wal_rotate_decision_as_is_ignore_pin s
          = ok pedra_aeneas_flush_kernel.WalRotateAction.KeepWal
        ↔ ¬(s.mem_empty = true ∧ s.imm_present = false
            ∧ s.parked_unflushed = false ∧ s.commit_inflight = false)) := by
  intro s
  simp only [pedra_aeneas_flush_kernel.wal_rotate_decision_as_is_ignore_pin]
  cases s.mem_empty <;> cases s.imm_present <;> cases s.pin_live <;>
    cases s.parked_unflushed <;> cases s.commit_inflight <;>
    simp

/-- Bridge to the REGISTERED rotate close
    (`try_rotate_step_rotates_iff_pins_clear_segment_live`, Flush.lean —
    the caller step of `try_rotate_wal` in db.rs): the registered close
    owns the decision→step direction; this corollary instantiates it
    with the record forall to own the record→step direction. Compose,
    not duplicate: the step fires `rotate_wal_now` EXACTLY on an
    all-clear record, no inflight recheck, and a live segment. -/
theorem try_rotate_step_rotates_iff_all_clear_record :
    ∀ (s : pedra_aeneas_flush_kernel.WalPinState)
      (recheck_inflight : Bool) (pos : Aeneas.Std.U64),
      (Aeneas.Std.bind (pedra_aeneas_flush_kernel.wal_rotate_decision s)
        (fun a =>
          match a with
          | pedra_aeneas_flush_kernel.WalRotateAction.RotateWal =>
              if recheck_inflight = true then ok false
              else Aeneas.Std.bind
                (pedra_aeneas_flush_kernel.wal_segment_is_empty pos)
                (fun e => ok (!e))
          | pedra_aeneas_flush_kernel.WalRotateAction.KeepWal => ok false))
        = ok true ↔
      (s.mem_empty = true ∧ s.imm_present = false ∧ s.pin_live = false
        ∧ s.parked_unflushed = false ∧ s.commit_inflight = false
        ∧ recheck_inflight = false
        ∧ pedra_aeneas_flush_kernel.wal_segment_is_empty pos = ok false) := by
  intro s recheck_inflight pos
  refine Iff.trans
    (try_rotate_step_rotates_iff_pins_clear_segment_live s recheck_inflight
      pos) ?_
  rw [(wal_rotate_decision_fate_forall s).1]
  constructor
  · rintro ⟨hrec, hr, he⟩
    exact ⟨hrec.1, hrec.2.1, hrec.2.2.1, hrec.2.2.2.1, hrec.2.2.2.2, hr, he⟩
  · rintro ⟨hm, hi, hp, hpa, hc, hr, he⟩
    exact ⟨⟨hm, hi, hp, hpa, hc⟩, hr, he⟩

/-! ### The concrete dentes — corollaries by instantiation (c) -/

/-- Off-lock dente, now a COROLLARY: failed WAL I/O does not publish,
    and an inflight commit keeps the WAL (flush must not truncate the
    writer's bytes). -/
theorem concurrent_publish_and_inflight_keep_wal :
    pedra_aeneas_group_commit_kernel.may_publish_group false = ok false
      ∧ pedra_aeneas_flush_kernel.wal_rotate_decision
          { mem_empty := true, imm_present := false, pin_live := false,
            parked_unflushed := false, commit_inflight := true }
        = ok pedra_aeneas_flush_kernel.WalRotateAction.KeepWal := by
  have hfor := wal_rotate_decision_fate_forall
    { mem_empty := true, imm_present := false, pin_live := false,
      parked_unflushed := false, commit_inflight := true }
  exact ⟨concurrent_publish_fate_forall false, hfor.2.mpr (by simp)⟩

/-- Other branch dente, now a COROLLARY: WAL I/O Ok may publish, and an
    all-clear idle pipeline rotates. -/
theorem concurrent_publish_ok_and_idle_rotates :
    pedra_aeneas_group_commit_kernel.may_publish_group true = ok true
      ∧ pedra_aeneas_flush_kernel.wal_rotate_decision
          { mem_empty := true, imm_present := false, pin_live := false,
            parked_unflushed := false, commit_inflight := false }
        = ok pedra_aeneas_flush_kernel.WalRotateAction.RotateWal := by
  have hfor := wal_rotate_decision_fate_forall
    { mem_empty := true, imm_present := false, pin_live := false,
      parked_unflushed := false, commit_inflight := false }
  exact ⟨concurrent_publish_fate_forall true, hfor.1.mpr (by simp)⟩

/-- AS-IS dente, now a COROLLARY: publish after failed WAL (the lie),
    but inflight STILL keeps — the pin-hole mutant does not drop
    `commit_inflight` from the keep-disjunction. -/
theorem concurrent_as_is_publish_lie_inflight_still_keeps :
    pedra_aeneas_group_commit_kernel.may_publish_group_as_is false = ok true
      ∧ pedra_aeneas_flush_kernel.wal_rotate_decision_as_is_ignore_pin
          { mem_empty := true, imm_present := false, pin_live := false,
            parked_unflushed := false, commit_inflight := true }
        = ok pedra_aeneas_flush_kernel.WalRotateAction.KeepWal := by
  have hfor := wal_rotate_decision_as_is_ignore_pin_fate_forall
    { mem_empty := true, imm_present := false, pin_live := false,
      parked_unflushed := false, commit_inflight := true }
  refine ⟨?_, hfor.2.mpr (by simp)⟩
  unfold pedra_aeneas_group_commit_kernel.may_publish_group_as_is
  rfl

/-- OCC client + publish dente, now a COROLLARY: inflight uses the
    published snap; failed WAL does not publish. -/
theorem occ_snap_published_and_no_publish_on_wal_fail :
    pedra_aeneas_flush_kernel.occ_snap_uses_published true = ok true
      ∧ pedra_aeneas_group_commit_kernel.may_publish_group false
        = ok false := by
  refine ⟨?_, concurrent_publish_fate_forall false⟩
  unfold pedra_aeneas_flush_kernel.occ_snap_uses_published
  rfl
