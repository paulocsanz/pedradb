-- Theorems over the Aeneas extract of production group_commit_kernel.rs
-- (RFC-0057 P2.1 / RFC-0058 P2.1): second machine (not the Verus twin).
--
-- The loop-free decisions (occ_conflict, the serialized mutant) get
-- universal closed-form theorems. The loop extracts (group_validate,
-- fence_publish_seq) are `partial_fixpoint` — irreducible to defeq — so
-- their example theorems bridge a compiler-checked `==` computation to
-- real equality through LawfulBEq (instances below; the universal
-- statements live in the Verus twin, verified by scripts/verus_group_commit.sh).
import Aeneas
import GroupCommitKernel
open Aeneas Std Result ControlFlow
open pedra_aeneas_group_commit_kernel

deriving instance BEq for OccMemberFate

/-- Structural lawfulness for the OCC member-fate enum (BEq is derived). -/
private instance : LawfulBEq OccMemberFate where
  eq_of_beq {a b} h := by
    cases a <;> cases b <;>
      first
        | rfl
        | exact absurd h (by decide)
  rfl {a} := by cases a <;> rfl

/-- Structural lawfulness for the error enum (BEq is derived). -/
private instance : LawfulBEq Error where
  eq_of_beq {a b} h := by
    cases a <;> cases b <;>
      first
        | rfl
        | exact absurd h (by decide)
  rfl {a} := by cases a <;> rfl

/-- Structural lawfulness for the Result monad (BEq is derived; the
`fail` constructor reuses the Error lawfulness above, the `ok`
constructor the element lawfulness, mixed constructors are decided by
`Bool.noConfusion`). -/
private instance [BEq α] [LawfulBEq α] : LawfulBEq (Result α) where
  eq_of_beq {a b} h := by
    cases a <;> cases b <;>
      first
        | rfl
        | exact congrArg Result.ok (LawfulBEq.eq_of_beq h)
        | exact congrArg Result.fail (LawfulBEq.eq_of_beq h)
        | exact Bool.noConfusion h
  rfl {a} := by
    cases a with
    | ok v => exact beq_self_eq_true v
    | fail e => exact beq_self_eq_true e
    | div => rfl

/-- First-committer-wins closed form: conflict iff the window
`(snap, last_seq]` is non-empty AND a touched key was written in it. -/
theorem occ_conflict_closed_form (snap last_seq : Std.U64)
    (touched : Bool) :
    occ_conflict snap last_seq touched =
      (if last_seq > snap then ok touched else ok false) := by
  unfold occ_conflict
  split <;> rfl

/-- The `last_seq == snap` fast path is sound: an empty window never
conflicts, whatever the scan said. -/
theorem fast_path_never_conflicts (snap : Std.U64) (touched : Bool) :
    occ_conflict snap snap touched = ok false := by
  rw [occ_conflict_closed_form]
  split
  · next h => exact absurd h (fun h' => Nat.lt_irrefl _ h')
  · rfl

/-- An inverted window (`last_seq <= snap`) is empty — nothing can be
written inside it (the kernel is total over all u64 inputs). -/
theorem inverted_window_never_conflicts :
    occ_conflict (9#u64) (7#u64) true = ok false := by
  rfl

/-- FENCE: the publish watermark is the max appended member sequence —
one `ok` value for the whole group (0 for an empty group). -/
theorem fence_is_max_member_seq :
    fence_publish_seq (⟨[5#u64, 2#u64, 9#u64, 4#u64], by native_decide⟩) = ok 9#u64 ∧
      fence_publish_seq (⟨[], by native_decide⟩) = ok 0#u64 ∧
      fence_publish_seq (⟨[3#u64], by native_decide⟩) = ok 3#u64 := by
  refine ⟨LawfulBEq.eq_of_beq (by native_decide), ?_, ?_⟩
  · exact LawfulBEq.eq_of_beq (by native_decide)
  · exact LawfulBEq.eq_of_beq (by native_decide)

/-- GROUP ATOMICITY (concrete): two members of one group, both read at
`snap == last_seq` — the group decides both clean (the group's own
writes do not exist at validation time), exactly as each member would
decide alone. -/
theorem group_members_are_simultaneous :
    group_validate
        (⟨[{ snap := 10#u64, touched_key_written_after := false },
           { snap := 10#u64, touched_key_written_after := false }],
          by native_decide⟩)
        (10#u64) =
      ok (⟨[false, false], by native_decide⟩ : alloc.vec.Vec Bool) ∧
      occ_conflict (10#u64) (10#u64) false = ok false := by
  refine ⟨LawfulBEq.eq_of_beq (by native_decide), ?_⟩
  rfl

/-- AS-IS teeth (RFC-0051 P1.3 planted-bug shape): the serialized
scheduler aborts the second same-group writer where the group form
commits it — simultaneity is exactly what the mutant loses. -/
theorem as_is_serialized_aborts_same_group_writer :
    occ_conflict (10#u64) (10#u64) true = ok false ∧
      occ_conflict_as_is_serialized (10#u64) (10#u64) (1#u64) true = ok true := by
  constructor <;> rfl

/-- ConcurrentDb `claim_lock_interleavings_proven` unfolds this: not a ∀π. -/
theorem lock_interleavings_not_a_theorem :
    lock_interleavings_admitted = ok false := by
  unfold lock_interleavings_admitted
  rfl

/-- AS-IS dente: a green publish is rounded to ∀ lock schedules. -/
theorem lock_interleavings_as_is_dente :
    lock_interleavings_admitted_as_is = ok true := by
  unfold lock_interleavings_admitted_as_is
  rfl

/-- ConcurrentDb publish gate: WAL I/O fail does not publish. -/
theorem may_publish_group_needs_wal_ok :
    may_publish_group false = ok false := by
  unfold may_publish_group
  rfl

/-- AS-IS dente: publish even if WAL I/O failed. -/
theorem may_publish_group_as_is_dente :
    may_publish_group_as_is false = ok true := by
  unfold may_publish_group_as_is
  rfl

/-- PCT depth 2 is not ∀ OS schedules of ConcurrentDb. -/
theorem forall_schedules_pct2_not_admitted :
    forall_schedules_admitted (2#u64) = ok false := by
  unfold forall_schedules_admitted
  rfl

/-- AS-IS dente: d≥2 is rounded to forall. -/
theorem forall_schedules_as_is_dente :
    forall_schedules_admitted_as_is (2#u64) = ok true := by
  unfold forall_schedules_admitted_as_is
  rfl

/-- ConcurrentDb lone_commit first-committer-wins: empty window never conflicts. -/
theorem concurrent_occ_empty_window :
    occ_conflict (10#u64) (10#u64) true = ok false := by
  unfold occ_conflict
  rfl

/-- ConcurrentDb `validate_occ_batch`: a lagging member (window `(7, 9]`,
    touched) conflicts. Unfolds `group_validate` (loop body calls
    `occ_conflict`) and the callee on the same input. -/
theorem group_validate_lagging_member_conflicts :
    group_validate
        (⟨[{ snap := 7#u64, touched_key_written_after := true }],
          by native_decide⟩)
        (9#u64) =
      ok (⟨[true], by native_decide⟩ : alloc.vec.Vec Bool) ∧
      occ_conflict (7#u64) (9#u64) true = ok true := by
  refine ⟨LawfulBEq.eq_of_beq (by native_decide), ?_⟩
  unfold occ_conflict
  rfl

/-- Second possibility of the same OCC edge: serialized scheduler conflicts
    where the group form on an empty window does not. -/
theorem group_occ_vs_serialized_same_input :
    occ_conflict (10#u64) (10#u64) true = ok false ∧
      occ_conflict_as_is_serialized (10#u64) (10#u64) (1#u64) true = ok true ∧
      group_validate
          (⟨[{ snap := 10#u64, touched_key_written_after := true }],
            by native_decide⟩)
          (10#u64) =
        ok (⟨[false], by native_decide⟩ : alloc.vec.Vec Bool) := by
  refine ⟨rfl, rfl, LawfulBEq.eq_of_beq (by native_decide)⟩

/-- `validate_occ_batch` caller: group_validate conflict ⇒ occ_member_fate Conflict. -/
theorem occ_member_fate_conflict_via_group_validate :
    group_validate
        (⟨[{ snap := 7#u64, touched_key_written_after := true }],
          by native_decide⟩)
        (9#u64) =
      ok (⟨[true], by native_decide⟩ : alloc.vec.Vec Bool) ∧
      occ_member_fate false true = ok OccMemberFate.Conflict := by
  constructor
  · exact LawfulBEq.eq_of_beq (by native_decide)
  · unfold occ_member_fate; rfl

/-- AS-IS dente: too-old + conflict still Ok. -/
theorem occ_member_fate_as_is_dente :
    occ_member_fate_as_is true true = ok OccMemberFate.Ok := by
  unfold occ_member_fate_as_is
  rfl

/-- `lone_commit` caller: occ_conflict ⇒ occ_member_fate Conflict. -/
theorem occ_member_fate_via_occ_conflict :
    occ_conflict (7#u64) (9#u64) true = ok true ∧
      occ_member_fate false true = ok OccMemberFate.Conflict := by
  constructor
  · unfold occ_conflict; rfl
  · unfold occ_member_fate; rfl

/-- N-way: three OccReads, one last_seq. Only the lagging member conflicts. -/
theorem group_validate_n3_one_lagging :
    group_validate
        (⟨[{ snap := 10#u64, touched_key_written_after := true },
           { snap := 10#u64, touched_key_written_after := true },
           { snap := 7#u64, touched_key_written_after := true }],
          by native_decide⟩)
        (10#u64) =
      ok (⟨[false, false, true], by native_decide⟩ : alloc.vec.Vec Bool) := by
  exact LawfulBEq.eq_of_beq (by native_decide)

/-- ConcurrentDb `validate_occ_batch` / `lone_commit` caller: `occ_batch_plan`
    unfolds `occ_member_fate` and `occ_conflict`. Lagging member is Conflict. -/
theorem occ_batch_plan_lagging_conflict :
    occ_batch_plan
        (⟨[false], by native_decide⟩)
        (⟨[{ snap := 7#u64, touched_key_written_after := true }],
          by native_decide⟩)
        (10#u64) =
      ok (⟨[OccMemberFate.Conflict], by native_decide⟩
        : alloc.vec.Vec OccMemberFate) ∧
      occ_member_fate false true = ok OccMemberFate.Conflict ∧
      occ_conflict (7#u64) (10#u64) true = ok true := by
  constructor
  · exact LawfulBEq.eq_of_beq (by native_decide)
  constructor
  · unfold occ_member_fate; rfl
  · unfold occ_conflict; rfl

/-- TooOld wins over Conflict on the same plan. -/
theorem occ_batch_plan_too_old_wins :
    occ_batch_plan
        (⟨[true], by native_decide⟩)
        (⟨[{ snap := 7#u64, touched_key_written_after := true }],
          by native_decide⟩)
        (10#u64) =
      ok (⟨[OccMemberFate.TooOld], by native_decide⟩
        : alloc.vec.Vec OccMemberFate) := by
  exact LawfulBEq.eq_of_beq (by native_decide)

/-- AS-IS dente: lagging member still Ok. -/
theorem occ_batch_plan_as_is_dente :
    occ_batch_plan_as_is
        (⟨[false], by native_decide⟩)
        (⟨[{ snap := 7#u64, touched_key_written_after := true }],
          by native_decide⟩)
        (10#u64) =
      ok (⟨[OccMemberFate.Ok], by native_decide⟩
        : alloc.vec.Vec OccMemberFate) := by
  exact LawfulBEq.eq_of_beq (by native_decide)

/-- Data-race token: exclusive mutate only while the write guard is held. -/
theorem rwlock_client_may_mutate_needs_write :
    rwlock_client_may_mutate false = ok false ∧
      rwlock_client_may_mutate true = ok true := by
  constructor
  · unfold rwlock_client_may_mutate; rfl
  · unfold rwlock_client_may_mutate; rfl

/-- AS-IS dente: mutate after dropping the write lock. -/
theorem rwlock_client_may_mutate_as_is_dente :
    rwlock_client_may_mutate_as_is false = ok true := by
  unfold rwlock_client_may_mutate_as_is
  rfl
