-- RFC-0225 P2: three machine-checked research-axis theorems over
-- registered iff atoms. Extracted bodies stay closed.
-- A1 write-path refinement (admission × WAL × torn-tail).
-- A3 lookup/scan get-filter: live ⇒ Value ∧ not hidden.
-- A6 OCC member fate ∀ of the plan the write group calls.
import Aeneas
import ComposeStorageWrite
import Merge
import Lookup
import GroupCommit
open Aeneas.Std Result
open pedra_aeneas_write_admission_kernel
open pedra_aeneas_lookup_kernel
open pedra_aeneas_group_commit_kernel

/-- A1: the rustc-linked storage write path (write_admit × wal_commit_plan
    × torn_tail_needs_cut) refines the spec that a record is recovered
    EXACTLY when admission is Ok, required sync did not fence, and the
    tail is not torn. Dual-unfold of the registered atoms; statement is
    `storage_write_path_recovered_iff`. -/
theorem pedra_refines :
    ∀ (mem_bytes : U64) (mem_armed : Bool) (mem_limit l0 : U64)
      (l0_armed : Bool) (l0_limit : U64)
      (need_sync sync_failed : Bool) (len last_good : U64) (v : Bool),
      (Aeneas.Std.bind
        (write_admit mem_bytes mem_armed mem_limit l0 l0_armed l0_limit)
        (fun adm =>
          match adm with
          | WriteAdmit.Ok =>
              Aeneas.Std.bind (wal_commit_plan need_sync sync_failed)
                (fun plan =>
                  match plan with
                  | WalCommitPlan.AppendSyncFence => ok false
                  | _ =>
                      Aeneas.Std.bind (torn_tail_needs_cut len last_good)
                        (fun cut => ok (!cut)))
          | WriteAdmit.StallMem => ok false
          | WriteAdmit.StallL0 => ok false) = ok v) ↔
        ((v = true ∧
            ¬(mem_armed = true ∧ mem_bytes >= mem_limit) ∧
            ¬(l0_armed = true ∧ l0 >= l0_limit) ∧
            ¬(need_sync = true ∧ sync_failed = true) ∧
            ¬(len > last_good))
          ∨ (v = false ∧
            ((mem_armed = true ∧ mem_bytes >= mem_limit)
              ∨ (l0_armed = true ∧ l0 >= l0_limit)
              ∨ (need_sync = true ∧ sync_failed = true)
              ∨ (len > last_good)))) :=
  storage_write_path_recovered_iff

/-- A3: lookup/scan isolation over the rustc-linked get-filter
    (`visible_at`) and the lookup tombstone plan (`point_tombstone_plan`).
    A live answer is isolated to an un-hidden Value; Deletion,
    RangeDeletion, and range-hidden versions never surface live, and the
    tombstone plan is ValueVisible exactly then. Registered iff only. -/
theorem confinement :
    ∀ (kind : pedra_aeneas_merge_kernel.key.ValueType) (range_hidden live : Bool)
      (plan : PointTombstonePlan),
      (pedra_aeneas_merge_kernel.merge.visible_at kind range_hidden = ok live ∧
          point_tombstone_plan range_hidden = ok plan) →
        ((live = true ∧
            kind = pedra_aeneas_merge_kernel.key.ValueType.Value ∧
            range_hidden = false ∧
            plan = PointTombstonePlan.ValueVisible) ∨
          (live = false ∧
            ((kind = pedra_aeneas_merge_kernel.key.ValueType.Deletion) ∨
              (kind = pedra_aeneas_merge_kernel.key.ValueType.RangeDeletion) ∨
              range_hidden = true))) := by
  intro kind range_hidden live plan ⟨hvis, hplan⟩
  have hv := (visible_at_fate_iff kind range_hidden live).mp hvis
  have hp := (point_tombstone_plan_fate_iff range_hidden plan).mp hplan
  rcases hv with ⟨hk, hlive⟩ | ⟨hkdel, hlivef⟩
  · subst hlive
    rcases hp with ⟨hhid, hsh⟩ | ⟨hopen, hvisp⟩
    · refine Or.inr ⟨?_, Or.inr (Or.inr hhid)⟩
      cases range_hidden
      · cases hhid
      · rfl
    · refine Or.inl ⟨?_, hk, hopen, hvisp⟩
      cases range_hidden
      · rfl
      · cases hopen
  · refine Or.inr ⟨hlivef, ?_⟩
    rcases hkdel with hdel | hrng
    · exact Or.inl hdel
    · exact Or.inr (Or.inl hrng)

/-- A6: ∀ over the rustc-linked write-group plan ConcurrentDb calls
    (`occ_conflict` then `occ_member_fate` — the per-member glue of
    `occ_batch_plan` / `validate_occ_batch`). Every member is TooOld,
    Conflict, or Ok EXACTLY along the registered atoms. -/
theorem concurrent_db_write_group_forall :
    ∀ (too_old_i : Bool) (snap last_seq : U64) (touched : Bool)
      (f : OccMemberFate),
      (Aeneas.Std.bind (occ_conflict snap last_seq touched)
        (occ_member_fate too_old_i) = ok f) ↔
        (∃ c, occ_conflict snap last_seq touched = ok c ∧
          ((too_old_i = true ∧ f = OccMemberFate.TooOld) ∨
            (¬(too_old_i = true) ∧ c = true ∧
              f = OccMemberFate.Conflict) ∨
            (¬(too_old_i = true) ∧ ¬(c = true) ∧
              f = OccMemberFate.Ok))) :=
  occ_batch_plan_member_fate_iff
