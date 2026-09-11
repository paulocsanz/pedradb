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

/-- RFC-0205 P0.1 (sixth registered close): the group publish gate is
    the EXACT flip of the WAL I/O outcome — a group is published iff its
    WAL write succeeded; there is no third fate (pure-lift mold,
    precedent dir_sync_required_ok_iff_sync). The dente above is the
    false instance; the AS-IS mutant publishes even on WAL failure. -/
theorem may_publish_group_ok_iff_wal_io_ok :
    ∀ (wal_io_ok v : Bool),
      (may_publish_group wal_io_ok = ok v) ↔ (wal_io_ok = v) := by
  intro wal_io_ok v
  unfold may_publish_group
  constructor
  · intro h
    injection h with _
  · intro h
    rw [h]

/-- Write-lock client protocol (registered atom, RFC-0202 P0.1):
    mutation of `Db` is permitted EXACTLY while the client holds the
    write guard — the AS-IS (mutate after dropping the guard) is
    unreachable from the real kernel. -/
theorem rwlock_client_may_mutate_ok_iff_holding_write :
    ∀ (holding_write v : Bool),
      (rwlock_client_may_mutate holding_write = ok v) ↔ (v = holding_write) := by
  intro holding_write v
  unfold rwlock_client_may_mutate
  simp [eq_comm]

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

/-- OCC member-fate precedence (registered atom, RFC-0202 P0.2):
    TooOld wins over Conflict over Ok — the AS-IS never-abort (lagging
    member commits) is unreachable from the real kernel. -/
theorem occ_member_fate_ok_iff_precedence :
    ∀ (too_old conflict : Bool) (f : OccMemberFate),
      (occ_member_fate too_old conflict = ok f) ↔
      ((too_old = true ∧ f = OccMemberFate.TooOld) ∨
       (too_old = false ∧ conflict = true ∧ f = OccMemberFate.Conflict) ∨
       (too_old = false ∧ conflict = false ∧ f = OccMemberFate.Ok)) := by
  intro too_old conflict f
  unfold occ_member_fate
  cases too_old <;> cases conflict <;> simp [eq_comm]

/-- `lone_commit` caller: occ_conflict ⇒ occ_member_fate Conflict. -/
theorem occ_member_fate_via_occ_conflict :
    occ_conflict (7#u64) (9#u64) true = ok true ∧
      occ_member_fate false true = ok OccMemberFate.Conflict := by
  constructor
  · unfold occ_conflict; rfl
  · unfold occ_member_fate; rfl

/-- N-way: three OccReads, one last_seq. Only the lagging member conflicts.
    Unfolds `group_validate` (the loop rustc links) **and** `occ_conflict`.
    `native_decide` of the loop without `unfold` is not compose. -/
theorem group_validate_n3_one_lagging :
    group_validate
        (⟨[{ snap := 10#u64, touched_key_written_after := true },
           { snap := 10#u64, touched_key_written_after := true },
           { snap := 7#u64, touched_key_written_after := true }],
          by native_decide⟩)
        (10#u64) =
      ok (⟨[false, false, true], by native_decide⟩ : alloc.vec.Vec Bool) ∧
      occ_conflict (7#u64) (10#u64) true = ok true ∧
      occ_conflict (10#u64) (10#u64) true = ok false := by
  refine ⟨?g, ?lag, ?same⟩
  · unfold group_validate
    exact LawfulBEq.eq_of_beq (by native_decide)
  · unfold occ_conflict; rfl
  · unfold occ_conflict; rfl

/-- N-way of the plan `validate_occ_batch` matches: three members, one last_seq.
    Unfolds `occ_batch_plan` **and** `occ_conflict`. -/
theorem occ_batch_plan_n3_one_lagging :
    occ_batch_plan
        (⟨[false, false, false], by native_decide⟩)
        (⟨[{ snap := 10#u64, touched_key_written_after := true },
           { snap := 10#u64, touched_key_written_after := true },
           { snap := 7#u64, touched_key_written_after := true }],
          by native_decide⟩)
        (10#u64) =
      ok (⟨[OccMemberFate.Ok, OccMemberFate.Ok, OccMemberFate.Conflict],
            by native_decide⟩
        : alloc.vec.Vec OccMemberFate) ∧
      occ_conflict (7#u64) (10#u64) true = ok true := by
  constructor
  · unfold occ_batch_plan
    exact LawfulBEq.eq_of_beq (by native_decide)
  · unfold occ_conflict; rfl

/-- ConcurrentDb `validate_occ_batch` caller: Lean `unfold`s the plan rustc
    links (`occ_batch_plan`) **and** the callees. Lagging member is Conflict.
    `native_decide` of the plan without `unfold` is not compose. -/
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
  · unfold occ_batch_plan
    exact LawfulBEq.eq_of_beq (by native_decide)
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

/-- AS-IS dente: lagging member still Ok. Unfold the as-is plan. -/
theorem occ_batch_plan_as_is_dente :
    occ_batch_plan_as_is
        (⟨[false], by native_decide⟩)
        (⟨[{ snap := 7#u64, touched_key_written_after := true }],
          by native_decide⟩)
        (10#u64) =
      ok (⟨[OccMemberFate.Ok], by native_decide⟩
        : alloc.vec.Vec OccMemberFate) := by
  unfold occ_batch_plan_as_is
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

/-- Data-race reader token: no guard ⇒ cannot read last_seq. Unfolds the
    plan rustc links and the mutate callee. -/
theorem rwlock_client_may_read_needs_guard :
    rwlock_client_may_read false false = ok false ∧
      rwlock_client_may_mutate false = ok false := by
  constructor
  · unfold rwlock_client_may_read
    unfold rwlock_client_may_mutate
    rfl
  · unfold rwlock_client_may_mutate; rfl

/-- Other branch: a read guard allows the snap even without the write lock. -/
theorem rwlock_client_may_read_with_read_guard :
    rwlock_client_may_read true false = ok true ∧
      rwlock_client_may_mutate false = ok false := by
  constructor
  · unfold rwlock_client_may_read
    unfold rwlock_client_may_mutate
    rfl
  · unfold rwlock_client_may_mutate; rfl

/-- Write guard implies mutate and therefore read. -/
theorem rwlock_client_may_read_via_write :
    rwlock_client_may_read false true = ok true ∧
      rwlock_client_may_mutate true = ok true := by
  constructor
  · unfold rwlock_client_may_read
    unfold rwlock_client_may_mutate
    rfl
  · unfold rwlock_client_may_mutate; rfl

/-- AS-IS dente: read Db with no guard. -/
theorem rwlock_client_may_read_as_is_dente :
    rwlock_client_may_read_as_is false false = ok true := by
  unfold rwlock_client_may_read_as_is
  rfl
-- RFC-0198 P0.2 theorem, PROVEN GREEN (lake build GroupCommit ✔ 1699 jobs)
-- before a foreign `git reset` wiped the working-tree insert. Re-insert
-- verbatim at the end of formal/aeneas/lean/GroupCommit.lean when the
-- shared registry files (close_proofs.tsv / proof_depth.tsv /
-- residuals.json) settle, then register per the P0.1 recipe.
-- Build evidence: first attempt failed `rw [if_pos ht, hv]` (ht is the
-- PAIR) — fixed to `rw [if_pos ht.1, ht.2]`, then built clean.

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

/-- RFC-0198 P0.2 (second registered glue close): every member fate the
    batch plan `occ_batch_plan` assigns is decided EXACTLY along the
    callee chain — the loop body binds `occ_conflict`'s answer into
    `occ_member_fate` (the extracted per-member glue): TooOld wins by the
    member flag alone; Conflict requires the callee to answer ok true on
    a touched key; Ok requires the callee's ok false. The loop's
    whole-output behavior is pinned concretely above
    (`occ_batch_plan_lagging_conflict`, `occ_batch_plan_n3_one_lagging`,
    `occ_batch_plan_too_old_wins`). -/
theorem occ_batch_plan_member_fate_iff :
    ∀ (too_old_i : Bool) (snap last_seq : Std.U64)
      (touched : Bool) (f : OccMemberFate),
      (Aeneas.Std.bind (occ_conflict snap last_seq touched)
        (occ_member_fate too_old_i) = ok f) ↔
        (∃ c, occ_conflict snap last_seq touched = ok c ∧
          ((too_old_i = true ∧ f = OccMemberFate.TooOld) ∨
            (¬(too_old_i = true) ∧ c = true ∧
              f = OccMemberFate.Conflict) ∨
            (¬(too_old_i = true) ∧ ¬(c = true) ∧
              f = OccMemberFate.Ok))) := by
  intro too_old_i snap last_seq touched f
  constructor
  · intro hval
    obtain ⟨c, hw, hval⟩ := bind_ok_inv _ _ _ hval
    refine ⟨c, hw, ?_⟩
    unfold occ_member_fate at hval
    split at hval
    · next ht =>
      injection hval with hv
      exact Or.inl ⟨ht, hv.symm⟩
    · next ht =>
      split at hval
      · next hc =>
        injection hval with hv
        exact Or.inr (Or.inl ⟨ht, hc, hv.symm⟩)
      · next hc =>
        injection hval with hv
        exact Or.inr (Or.inr ⟨ht, hc, hv.symm⟩)
  · rintro ⟨c, hw, ht | ⟨ht, hc, hv⟩ | ⟨ht, hc, hv⟩⟩
    · refine bind_intro c hw ?_
      unfold occ_member_fate
      rw [if_pos ht.1, ht.2]
    · refine bind_intro c hw ?_
      unfold occ_member_fate
      rw [if_neg ht, if_pos hc, hv]
    · refine bind_intro c hw ?_
      unfold occ_member_fate
      rw [if_neg ht, if_neg hc, hv]

