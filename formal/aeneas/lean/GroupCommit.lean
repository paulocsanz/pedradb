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
open Aeneas Std Result
open pedra_aeneas_group_commit_kernel

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
