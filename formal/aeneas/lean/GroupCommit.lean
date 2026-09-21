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

/-- AS-IS tooth: a green publish is rounded to ∀ lock schedules. -/
theorem lock_interleavings_as_is_tooth :
    lock_interleavings_admitted_as_is = ok true := by
  unfold lock_interleavings_admitted_as_is
  rfl

/-- ConcurrentDb publish gate: WAL I/O fail does not publish. -/
theorem may_publish_group_needs_wal_ok :
    may_publish_group false = ok false := by
  unfold may_publish_group
  rfl

/-- AS-IS tooth: publish even if WAL I/O failed. -/
theorem may_publish_group_as_is_tooth :
    may_publish_group_as_is false = ok true := by
  unfold may_publish_group_as_is
  rfl

/-- RFC-0205 P0.1 (sixth registered close): the group publish gate is
    the EXACT flip of the WAL I/O outcome — a group is published iff its
    WAL write succeeded; there is no third fate (pure-lift mold,
    precedent dir_sync_required_ok_iff_sync). The tooth above is the
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

/-- Always refuse: no PCT depth is ∀ OS schedules. Flag stays false. -/
theorem forall_schedules_admitted_fate_iff :
    ∀ (d : U64) (v : Bool),
      (forall_schedules_admitted d = ok v) ↔ (v = false) := by
  intro d v
  unfold forall_schedules_admitted
  constructor
  · intro h; injection h with hv; exact hv.symm
  · intro h; subst h; rfl

/-- Campaign default PCT depth is 2 (RFC-0070 does not raise it). -/
theorem pct_campaign_default_depth_fate_iff :
    ∀ (d : U64),
      (pct_campaign_default_depth = ok d) ↔ (d = 2#u64) := by
  intro d
  unfold pct_campaign_default_depth
  constructor
  · intro h; injection h with hv; exact hv.symm
  · intro h; subst h; rfl

/-- Claim that 0070 raised the default PCT depth: always false. -/
theorem default_pct_depth_raised_fate_iff :
    ∀ (v : Bool),
      (default_pct_depth_raised = ok v) ↔ (v = false) := by
  intro v
  unfold default_pct_depth_raised
  constructor
  · intro h; injection h with hv; exact hv.symm
  · intro h; subst h; rfl

/-- Lock-schedule ∀π is not a theorem. Flag stays false. -/
theorem lock_interleavings_admitted_fate_iff :
    ∀ (v : Bool),
      (lock_interleavings_admitted = ok v) ↔ (v = false) := by
  intro v
  unfold lock_interleavings_admitted
  constructor
  · intro h; injection h with hv; exact hv.symm
  · intro h; subst h; rfl

/-- fdatasync rc==0 is not media durability. Flag stays false. -/
theorem media_durable_admitted_fate_iff :
    ∀ (fsync_ok v : Bool),
      (media_durable_admitted fsync_ok = ok v) ↔ (v = false) := by
  intro fsync_ok v
  unfold media_durable_admitted
  constructor
  · intro h; injection h with hv; exact hv.symm
  · intro h; subst h; rfl

/-- Stacking two fsync-liar boxes is not a campaign. Flag stays false. -/
theorem stacked_fsync_liars_admitted_fate_iff :
    ∀ (lying det_io v : Bool),
      (stacked_fsync_liars_admitted lying det_io = ok v) ↔ (v = false) := by
  intro lying det_io v
  unfold stacked_fsync_liars_admitted
  constructor
  · intro h; injection h with hv; exact hv.symm
  · intro h; subst h; rfl

/-- Closing the lying-fsync model does not invent a TCG guest. -/
theorem fsync_lie_closes_tcg_guest_fate_iff :
    ∀ (v : Bool),
      (fsync_lie_closes_tcg_guest = ok v) ↔ (v = false) := by
  intro v
  unfold fsync_lie_closes_tcg_guest
  constructor
  · intro h; injection h with hv; exact hv.symm
  · intro h; subst h; rfl

/-- AS-IS tooth: d≥2 is rounded to forall. -/
theorem forall_schedules_as_is_tooth :
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

/-- AS-IS tooth: too-old + conflict still Ok. -/
theorem occ_member_fate_as_is_tooth :
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

/-- AS-IS tooth: lagging member still Ok. Unfold the as-is plan. -/
theorem occ_batch_plan_as_is_tooth :
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

/-- AS-IS tooth: mutate after dropping the write lock. -/
theorem rwlock_client_may_mutate_as_is_tooth :
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

/-- AS-IS tooth: read Db with no guard. -/
theorem rwlock_client_may_read_as_is_tooth :
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

/-- RFC-0213 P1.2 (sixth registered atom; close→atom ladder, precedent
    wal_commit_plan): the WHOLE extracted plan `occ_batch_plan` decides
    EXACTLY along the min-length route — n is the shorter input (the only
    branch the extract takes) and every member fate comes from the
    extracted loop `occ_batch_plan_loop` seeded with the empty capacity-n
    vector. The RFC-0198 close pinned the per-member glue
    (occ_batch_plan_member_fate_iff above); this iff pins the extract
    itself — the batch fate is the loop's measured answer, not data
    folklore. -/
theorem occ_batch_plan_fate_iff :
    ∀ (too_old : Aeneas.Std.Slice Bool) (reads : Aeneas.Std.Slice OccRead)
      (last_seq : Std.U64) (v : alloc.vec.Vec OccMemberFate),
      (occ_batch_plan too_old reads last_seq = ok v) ↔
        ∃ (n : Std.Usize),
          ((Slice.len too_old <= Slice.len reads ∧ n = Slice.len too_old) ∨
            (¬(Slice.len too_old <= Slice.len reads) ∧ n = Slice.len reads)) ∧
          occ_batch_plan_loop too_old reads last_seq n
            (alloc.vec.Vec.with_capacity OccMemberFate n) 0#usize = ok v := by
  intro too_old reads last_seq v
  constructor
  · intro hval
    unfold occ_batch_plan at hval
    obtain ⟨n, hn, hval⟩ := bind_ok_inv _ _ _ hval
    refine ⟨n, ?_, hval⟩
    split at hn
    · next hle =>
      injection hn with hn'
      exact Or.inl ⟨hle, hn'.symm⟩
    · next hle =>
      injection hn with hn'
      exact Or.inr ⟨hle, hn'.symm⟩
  · rintro ⟨n, (⟨hle, hn⟩ | ⟨hle, hn⟩), hloop⟩
    · unfold occ_batch_plan
      refine bind_intro n ?_ hloop
      rw [if_pos hle, hn]
    · unfold occ_batch_plan
      refine bind_intro n ?_ hloop
      rw [if_neg hle, hn]

/-- RFC-0218 P0.1 1/4 (atom `catalog:group_commit`, entrada
    `occ_conflict`): o veredito OCC first-committer-wins é exatamente a
    janela — o conflito é ok EXATAMENTE quando a janela `(snap,
    last_seq]` é não-vazia E a resposta é a flag tocada, ou a janela é
    vazia e a resposta é false; sem terceiro destino. O AS-IS
    serializado planta o tooth oposto no mesmo writer do grupo. -/
theorem occ_conflict_fate_iff :
    ∀ (snap last_seq : Std.U64) (touched v : Bool),
      (occ_conflict snap last_seq touched = ok v) ↔
        ((last_seq > snap ∧ v = touched) ∨
          (¬(last_seq > snap) ∧ v = false)) := by
  intro snap last_seq touched v
  constructor
  · intro hval
    rw [occ_conflict_closed_form] at hval
    split at hval
    · next hgt => exact Or.inl ⟨hgt, (Result.ok.inj hval).symm⟩
    · next hgt => exact Or.inr ⟨hgt, (Result.ok.inj hval).symm⟩
  · rintro (⟨hgt, hv⟩ | ⟨hgt, hv⟩)
    · rw [occ_conflict_closed_form, if_pos hgt, hv]
    · rw [occ_conflict_closed_form, if_neg hgt, hv]

/-- RFC-0218 P0.1 2/4 (atom `catalog:fsync_promote`, entrada
    `fsync_promotes_pending`): pending vira durável EXATAMENTE quando o
    OS/Env é honesto — o corpo é o lift puro `ok os_honest`; sem
    terceiro destino. O AS-IS promove mesmo com fsync mentiroso (tooth
    RFC-0078: planta `fsync_promotes_pending_on_live_sim_is_not_ok`). -/
theorem fsync_promotes_pending_fate_iff :
    ∀ (os_honest v : Bool),
      (fsync_promotes_pending os_honest = ok v) ↔ (os_honest = v) := by
  intro os_honest v
  constructor
  · intro hval
    exact Result.ok.inj hval
  · intro h
    unfold fsync_promotes_pending
    rw [h]


/-! ### RFC-0218 P0.1 3/4 — `fence_publish_seq` (atom `catalog:group_fence`)

O fate do fence como cadeia (molde Form/DecodeFate): combustível =
membros restantes; cada passo `cont` consome exatamente um membro (i'
= i+1 ≤ len) e o fim é o `done` exato em i = len com best = v. -/

/-- O `+1#usize` do corpo vale exatamente `↑i + 1` em Nat. -/
private theorem gc_usize_succ_val (i i1 : Usize) (h : (i + 1#usize) = ok i1) :
    (↑i1 : Nat) = (↑i : Nat) + 1 := by
  have he := UScalar.add_equiv i 1#usize
  rw [h] at he
  dsimp only at he
  exact he.2.1

/-- No fim (i = len) o corpo devolve exatamente `done best`. -/
private theorem fence_body_at_end (member_seqs : Aeneas.Std.Slice Std.U64)
    (best : Std.U64) (i : Usize)
    (hlen : (↑i : Nat) = (member_seqs.val).length) :
    fence_publish_seq_loop.body member_seqs best i
      = ok (ControlFlow.done best) := by
  have hge : ¬ (i < Aeneas.Std.Slice.len member_seqs) := by
    intro hlt
    have hn0 := (UScalar.lt_equiv i (Aeneas.Std.Slice.len member_seqs)).mp hlt
    rw [Aeneas.Std.Slice.len_val] at hn0
    rw [hlen] at hn0
    exact absurd hn0 (Nat.lt_irrefl _)
  unfold fence_publish_seq_loop.body
  dsimp +zeta only
  rw [if_neg hge]

/-- No fim o corpo nunca dá cont. -/
private theorem fence_body_no_cont_at_end (member_seqs : Aeneas.Std.Slice Std.U64)
    (best : Std.U64) (i : Usize) (st : Std.U64 × Usize)
    (hlen : (↑i : Nat) = (member_seqs.val).length)
    (hB : fence_publish_seq_loop.body member_seqs best i
            = ok (ControlFlow.cont st)) : False := by
  have hge : ¬ (i < Aeneas.Std.Slice.len member_seqs) := by
    intro hlt
    have hn0 := (UScalar.lt_equiv i (Aeneas.Std.Slice.len member_seqs)).mp hlt
    rw [Aeneas.Std.Slice.len_val] at hn0
    rw [hlen] at hn0
    exact absurd hn0 (Nat.lt_irrefl _)
  unfold fence_publish_seq_loop.body at hB
  dsimp +zeta only at hB
  rw [if_neg hge] at hB
  injection hB with hB2
  contradiction

/-- Sob i < len o corpo é exatamente `cont (best', i')` com o índice
estritamente crescente e limitado — o max interno é consumido pelo
bind_ok_inv sem precisar de ramo (ambas as folhas são ok). -/
private theorem fence_body_inv (member_seqs : Aeneas.Std.Slice Std.U64)
    (best : Std.U64) (i : Usize)
    (hlt : (↑i : Nat) < (member_seqs.val).length)
    (cf : ControlFlow (Std.U64 × Usize) Std.U64)
    (hB : fence_publish_seq_loop.body member_seqs best i = ok cf) :
    ∃ (best' : Std.U64) (i' : Usize),
      cf = ControlFlow.cont (best', i') ∧
        (↑i : Nat) < (↑i' : Nat) ∧ (↑i' : Nat) ≤ (member_seqs.val).length := by
  have hlt' : i < Aeneas.Std.Slice.len member_seqs := by
    refine (UScalar.lt_equiv i (Aeneas.Std.Slice.len member_seqs)).mpr ?_
    rw [Aeneas.Std.Slice.len_val]
    exact hlt
  unfold fence_publish_seq_loop.body at hB
  dsimp +zeta only at hB
  rw [if_pos hlt'] at hB
  obtain ⟨i2, hi2, hB⟩ := bind_ok_inv _ _ _ hB
  obtain ⟨best1, hbest1, hB⟩ := bind_ok_inv _ _ _ hB
  obtain ⟨i3, hi3, hB⟩ := bind_ok_inv _ _ _ hB
  have hv := gc_usize_succ_val i i3 hi3
  exact ⟨best1, i3, (Result.ok.inj hB).symm, by omega, by omega⟩

/-- Payload de um cont sob i < len progride: i < i' ≤ len. -/
private theorem fence_body_cont_progress (member_seqs : Aeneas.Std.Slice Std.U64)
    (best : Std.U64) (i : Usize) (best' : Std.U64) (i' : Usize)
    (hlt : (↑i : Nat) < (member_seqs.val).length)
    (hB : fence_publish_seq_loop.body member_seqs best i
            = ok (ControlFlow.cont (best', i'))) :
    (↑i : Nat) < (↑i' : Nat) ∧ (↑i' : Nat) ≤ (member_seqs.val).length := by
  obtain ⟨best2, i2, hcf, hlt2, hle2⟩ :=
    fence_body_inv member_seqs best i hlt (ControlFlow.cont (best', i')) hB
  have hp := ControlFlow.cont.inj hcf
  obtain ⟨-, hii⟩ := Prod.mk.inj hp
  subst hii
  exact ⟨hlt2, hle2⟩

/-- Done só no fim, com o best intacto. -/
private theorem fence_body_done_end (member_seqs : Aeneas.Std.Slice Std.U64)
    (best : Std.U64) (i : Usize) (r : Std.U64)
    (hle : (↑i : Nat) ≤ (member_seqs.val).length)
    (hB : fence_publish_seq_loop.body member_seqs best i
            = ok (ControlFlow.done r)) :
    (↑i : Nat) = (member_seqs.val).length ∧ best = r := by
  by_cases hlt : (↑i : Nat) < (member_seqs.val).length
  · obtain ⟨best2, i2, hcf, -, -⟩ :=
      fence_body_inv member_seqs best i hlt (ControlFlow.done r) hB
    exact absurd hcf (by intro hh; contradiction)
  · have hlen : (↑i : Nat) = (member_seqs.val).length := by omega
    refine ⟨hlen, ?_⟩
    have h := (fence_body_at_end member_seqs best i hlen).symm.trans hB
    exact ControlFlow.done.inj (Result.ok.inj h)

/-- O fate do fence como cadeia: combustível = membros restantes. -/
private def FenceFate (member_seqs : Aeneas.Std.Slice Std.U64) :
    Nat → Std.U64 → Usize → Std.U64 → Prop
  | 0, best, i, v =>
      (↑i : Nat) = (member_seqs.val).length ∧ best = v
  | fuel + 1, best, i, v =>
      (∃ (best' : Std.U64) (i' : Usize),
          fence_publish_seq_loop.body member_seqs best i
            = ok (ControlFlow.cont (best', i')) ∧
            FenceFate member_seqs fuel best' i' v) ∨
        ((↑i : Nat) = (member_seqs.val).length ∧ best = v)

/-- O fate do loop por indução no combustível. -/
private theorem fence_publish_seq_loop_fate (member_seqs : Aeneas.Std.Slice Std.U64) :
    ∀ (fuel : Nat) (best : Std.U64) (i : Usize),
      (↑i : Nat) ≤ (member_seqs.val).length →
      (member_seqs.val).length - (↑i : Nat) ≤ fuel →
      ∀ v : Std.U64,
        (fence_publish_seq_loop member_seqs best i = ok v) ↔
          FenceFate member_seqs fuel best i v := by
  intro fuel
  induction fuel with
  | zero =>
    intro best i hile hfuel v
    have hlen : (↑i : Nat) = (member_seqs.val).length := by omega
    constructor
    · intro h
      refine ⟨hlen, ?_⟩
      unfold fence_publish_seq_loop at h
      rw [loop.eq_def] at h
      dsimp only at h
      cases hB : fence_publish_seq_loop.body member_seqs best i with
      | ok cf =>
        cases cf with
        | done r =>
          rw [hB] at h
          dsimp only at h
          rw [Result.ok.inj h] at hB
          exact (fence_body_done_end member_seqs best i v hile hB).2
        | cont st =>
          exact absurd hB (fence_body_no_cont_at_end member_seqs best i st hlen)
      | fail e =>
        rw [hB] at h
        dsimp only at h
        exact absurd h (by simp)
      | div =>
        rw [hB] at h
        dsimp only at h
        exact absurd h (by simp)
    · rintro ⟨-, hbest⟩
      unfold fence_publish_seq_loop
      rw [loop.eq_def]
      dsimp only
      rw [fence_body_at_end member_seqs best i hlen, hbest]
  | succ fuel ih =>
    intro best i hile hfuel v
    unfold fence_publish_seq_loop
    rw [loop.eq_def]
    dsimp only
    cases hB : fence_publish_seq_loop.body member_seqs best i with
    | ok cf =>
      cases cf with
      | cont st =>
        obtain ⟨best', i'⟩ := st
        dsimp only
        by_cases hlt : (↑i : Nat) < (member_seqs.val).length
        · obtain ⟨hprog1, hprog2⟩ :=
            fence_body_cont_progress member_seqs best i best' i' hlt hB
          constructor
          · intro h
            exact Or.inl ⟨best', i', hB,
              (ih best' i' hprog2 (by omega) v).mp h⟩
          · rintro (⟨best2, i2, hbody, hfate⟩ | ⟨hlen, hbest⟩)
            · have hu : ControlFlow.cont (best', i')
                  = ControlFlow.cont (best2, i2) :=
                  Result.ok.inj (hB.symm.trans hbody)
              obtain ⟨hbb, hii⟩ := Prod.mk.inj (ControlFlow.cont.inj hu)
              subst hbb
              subst hii
              exact (ih best' i' hprog2 (by omega) v).mpr hfate
            · exact absurd hlt (by omega)
        · have hlen : (↑i : Nat) = (member_seqs.val).length := by omega
          exact absurd hB (fence_body_no_cont_at_end member_seqs best i (best', i') hlen)
      | done r =>
        dsimp only
        obtain ⟨hlen, hbest⟩ := fence_body_done_end member_seqs best i r hile hB
        constructor
        · intro h
          have hrv : r = v := Result.ok.inj h
          exact Or.inr ⟨hlen, hrv ▸ hbest⟩
        · rintro (⟨best2, i2, hbody, -⟩ | ⟨hlen2, hbest2⟩)
          · have hne := hbody.symm.trans hB
            injection hne with hne2
            contradiction
          · exact congrArg ok (hbest.symm.trans hbest2)
    | fail e =>
      dsimp only
      constructor
      · intro h
        exact absurd h (by simp)
      · rintro (⟨best2, i2, hbody, -⟩ | ⟨hlen2, hbest2⟩)
        · exact absurd (hbody.symm.trans hB) (by simp)
        · exact absurd ((fence_body_at_end member_seqs best i hlen2).symm.trans hB) (by simp)
    | div =>
      dsimp only
      constructor
      · intro h
        exact absurd h (by simp)
      · rintro (⟨best2, i2, hbody, -⟩ | ⟨hlen2, hbest2⟩)
        · exact absurd (hbody.symm.trans hB) (by simp)
        · exact absurd ((fence_body_at_end member_seqs best i hlen2).symm.trans hB) (by simp)

/-- RFC-0218 P0.1 3/4 (atom `catalog:group_fence`, entrada
    `fence_publish_seq`): o watermark de publish do grupo é exatamente a
    cadeia citada do loop extraído — cada passo lê um membro
    (`Slice.index_usize`), atualiza o máximo e avança i estritamente; o
    fim é `i = len` com o máximo acumulado `best = v`; sem terceiro
    destino. O AS-IS publica o primeiro membro e ignora o resto
    (tooth plantado). -/
theorem fence_publish_seq_fate_iff :
    ∀ (member_seqs : Aeneas.Std.Slice Std.U64) (v : Std.U64),
      (fence_publish_seq member_seqs = ok v) ↔
        FenceFate member_seqs (member_seqs.val).length 0#u64 0#usize v := by
  intro member_seqs v
  have hloop : fence_publish_seq member_seqs
      = fence_publish_seq_loop member_seqs 0#u64 0#usize := rfl
  rw [hloop]
  exact fence_publish_seq_loop_fate member_seqs (member_seqs.val).length _ 0#usize
    (Nat.zero_le _) (Nat.sub_le _ _) v

/-! ### RFC-0218 P0.1 4/4 — `group_validate` (atom `catalog:group_validate`)

Mesmo molde do fence: cada passo `cont` lê um OccRead, decide pelo
`occ_conflict` extraído (candidato a atom 1/4), empurra no out e avança
i estritamente; o fim é `i = len` com out = v. -/

/-- No fim (i = len) o corpo devolve exatamente `done out`. -/
private theorem gv_body_at_end (reads : Aeneas.Std.Slice OccRead)
    (last_seq : Std.U64) (out : alloc.vec.Vec Bool) (i : Usize)
    (hlen : (↑i : Nat) = (reads.val).length) :
    group_validate_loop.body reads last_seq out i
      = ok (ControlFlow.done out) := by
  have hge : ¬ (i < Aeneas.Std.Slice.len reads) := by
    intro hlt
    have hn0 := (UScalar.lt_equiv i (Aeneas.Std.Slice.len reads)).mp hlt
    rw [Aeneas.Std.Slice.len_val] at hn0
    rw [hlen] at hn0
    exact absurd hn0 (Nat.lt_irrefl _)
  unfold group_validate_loop.body
  dsimp +zeta only
  rw [if_neg hge]

/-- No fim o corpo nunca dá cont. -/
private theorem gv_body_no_cont_at_end (reads : Aeneas.Std.Slice OccRead)
    (last_seq : Std.U64) (out : alloc.vec.Vec Bool) (i : Usize)
    (st : alloc.vec.Vec Bool × Usize)
    (hlen : (↑i : Nat) = (reads.val).length)
    (hB : group_validate_loop.body reads last_seq out i
            = ok (ControlFlow.cont st)) : False := by
  have hge : ¬ (i < Aeneas.Std.Slice.len reads) := by
    intro hlt
    have hn0 := (UScalar.lt_equiv i (Aeneas.Std.Slice.len reads)).mp hlt
    rw [Aeneas.Std.Slice.len_val] at hn0
    rw [hlen] at hn0
    exact absurd hn0 (Nat.lt_irrefl _)
  unfold group_validate_loop.body at hB
  dsimp +zeta only at hB
  rw [if_neg hge] at hB
  injection hB with hB2
  contradiction

/-- Sob i < len o corpo é exatamente `cont (out', i')` com o índice
estritamente crescente e limitado — index/occ_conflict/push consumidos
pelos bind_ok_inv (todos os ramos ok). -/
private theorem gv_body_inv (reads : Aeneas.Std.Slice OccRead)
    (last_seq : Std.U64) (out : alloc.vec.Vec Bool) (i : Usize)
    (hlt : (↑i : Nat) < (reads.val).length)
    (cf : ControlFlow (alloc.vec.Vec Bool × Usize) (alloc.vec.Vec Bool))
    (hB : group_validate_loop.body reads last_seq out i = ok cf) :
    ∃ (out' : alloc.vec.Vec Bool) (i' : Usize),
      cf = ControlFlow.cont (out', i') ∧
        (↑i : Nat) < (↑i' : Nat) ∧ (↑i' : Nat) ≤ (reads.val).length := by
  have hlt' : i < Aeneas.Std.Slice.len reads := by
    refine (UScalar.lt_equiv i (Aeneas.Std.Slice.len reads)).mpr ?_
    rw [Aeneas.Std.Slice.len_val]
    exact hlt
  unfold group_validate_loop.body at hB
  dsimp +zeta only at hB
  rw [if_pos hlt'] at hB
  obtain ⟨or, hor, hB⟩ := bind_ok_inv _ _ _ hB
  obtain ⟨b, hb, hB⟩ := bind_ok_inv _ _ _ hB
  obtain ⟨out1, hout1, hB⟩ := bind_ok_inv _ _ _ hB
  obtain ⟨i2, hi2, hB⟩ := bind_ok_inv _ _ _ hB
  have hv := gc_usize_succ_val i i2 hi2
  exact ⟨out1, i2, (Result.ok.inj hB).symm, by omega, by omega⟩

/-- Payload de um cont sob i < len progride: i < i' ≤ len. -/
private theorem gv_body_cont_progress (reads : Aeneas.Std.Slice OccRead)
    (last_seq : Std.U64) (out : alloc.vec.Vec Bool) (i : Usize)
    (out' : alloc.vec.Vec Bool) (i' : Usize)
    (hlt : (↑i : Nat) < (reads.val).length)
    (hB : group_validate_loop.body reads last_seq out i
            = ok (ControlFlow.cont (out', i'))) :
    (↑i : Nat) < (↑i' : Nat) ∧ (↑i' : Nat) ≤ (reads.val).length := by
  obtain ⟨out2, i2, hcf, hlt2, hle2⟩ :=
    gv_body_inv reads last_seq out i hlt (ControlFlow.cont (out', i')) hB
  have hp := ControlFlow.cont.inj hcf
  obtain ⟨-, hii⟩ := Prod.mk.inj hp
  subst hii
  exact ⟨hlt2, hle2⟩

/-- Done só no fim, com o out intacto. -/
private theorem gv_body_done_end (reads : Aeneas.Std.Slice OccRead)
    (last_seq : Std.U64) (out : alloc.vec.Vec Bool) (i : Usize)
    (r : alloc.vec.Vec Bool)
    (hle : (↑i : Nat) ≤ (reads.val).length)
    (hB : group_validate_loop.body reads last_seq out i
            = ok (ControlFlow.done r)) :
    (↑i : Nat) = (reads.val).length ∧ out = r := by
  by_cases hlt : (↑i : Nat) < (reads.val).length
  · obtain ⟨out2, i2, hcf, -, -⟩ :=
      gv_body_inv reads last_seq out i hlt (ControlFlow.done r) hB
    exact absurd hcf (by intro hh; contradiction)
  · have hlen : (↑i : Nat) = (reads.val).length := by omega
    refine ⟨hlen, ?_⟩
    have h := (gv_body_at_end reads last_seq out i hlen).symm.trans hB
    exact ControlFlow.done.inj (Result.ok.inj h)

/-- O fate da validação como cadeia: combustível = membros restantes. -/
private def ValidateFate (reads : Aeneas.Std.Slice OccRead)
    (last_seq : Std.U64) :
    Nat → alloc.vec.Vec Bool → Usize → alloc.vec.Vec Bool → Prop
  | 0, out, i, v =>
      (↑i : Nat) = (reads.val).length ∧ out = v
  | fuel + 1, out, i, v =>
      (∃ (out' : alloc.vec.Vec Bool) (i' : Usize),
          group_validate_loop.body reads last_seq out i
            = ok (ControlFlow.cont (out', i')) ∧
            ValidateFate reads last_seq fuel out' i' v) ∨
        ((↑i : Nat) = (reads.val).length ∧ out = v)

/-- O fate do loop por indução no combustível. -/
private theorem group_validate_loop_fate (reads : Aeneas.Std.Slice OccRead)
    (last_seq : Std.U64) :
    ∀ (fuel : Nat) (out : alloc.vec.Vec Bool) (i : Usize),
      (↑i : Nat) ≤ (reads.val).length →
      (reads.val).length - (↑i : Nat) ≤ fuel →
      ∀ v : alloc.vec.Vec Bool,
        (group_validate_loop reads last_seq out i = ok v) ↔
          ValidateFate reads last_seq fuel out i v := by
  intro fuel
  induction fuel with
  | zero =>
    intro out i hile hfuel v
    have hlen : (↑i : Nat) = (reads.val).length := by omega
    constructor
    · intro h
      refine ⟨hlen, ?_⟩
      unfold group_validate_loop at h
      rw [loop.eq_def] at h
      dsimp only at h
      cases hB : group_validate_loop.body reads last_seq out i with
      | ok cf =>
        cases cf with
        | done r =>
          rw [hB] at h
          dsimp only at h
          rw [Result.ok.inj h] at hB
          exact (gv_body_done_end reads last_seq out i v hile hB).2
        | cont st =>
          exact absurd hB (gv_body_no_cont_at_end reads last_seq out i st hlen)
      | fail e =>
        rw [hB] at h
        dsimp only at h
        exact absurd h (by simp)
      | div =>
        rw [hB] at h
        dsimp only at h
        exact absurd h (by simp)
    · rintro ⟨-, hout⟩
      unfold group_validate_loop
      rw [loop.eq_def]
      dsimp only
      rw [gv_body_at_end reads last_seq out i hlen, hout]
  | succ fuel ih =>
    intro out i hile hfuel v
    unfold group_validate_loop
    rw [loop.eq_def]
    dsimp only
    cases hB : group_validate_loop.body reads last_seq out i with
    | ok cf =>
      cases cf with
      | cont st =>
        obtain ⟨out', i'⟩ := st
        dsimp only
        by_cases hlt : (↑i : Nat) < (reads.val).length
        · obtain ⟨hprog1, hprog2⟩ :=
            gv_body_cont_progress reads last_seq out i out' i' hlt hB
          constructor
          · intro h
            exact Or.inl ⟨out', i', hB,
              (ih out' i' hprog2 (by omega) v).mp h⟩
          · rintro (⟨out2, i2, hbody, hfate⟩ | ⟨hlen, hout⟩)
            · have hu : ControlFlow.cont (out', i')
                  = ControlFlow.cont (out2, i2) :=
                  Result.ok.inj (hB.symm.trans hbody)
              obtain ⟨hoo, hii⟩ := Prod.mk.inj (ControlFlow.cont.inj hu)
              subst hoo
              subst hii
              exact (ih out' i' hprog2 (by omega) v).mpr hfate
            · exact absurd hlt (by omega)
        · have hlen : (↑i : Nat) = (reads.val).length := by omega
          exact absurd hB (gv_body_no_cont_at_end reads last_seq out i (out', i') hlen)
      | done r =>
        dsimp only
        obtain ⟨hlen, hout⟩ := gv_body_done_end reads last_seq out i r hile hB
        constructor
        · intro h
          have hrv : r = v := Result.ok.inj h
          exact Or.inr ⟨hlen, hrv ▸ hout⟩
        · rintro (⟨out2, i2, hbody, -⟩ | ⟨hlen2, hout2⟩)
          · have hne := hbody.symm.trans hB
            injection hne with hne2
            contradiction
          · exact congrArg ok (hout.symm.trans hout2)
    | fail e =>
      dsimp only
      constructor
      · intro h
        exact absurd h (by simp)
      · rintro (⟨out2, i2, hbody, -⟩ | ⟨hlen2, hout2⟩)
        · exact absurd (hbody.symm.trans hB) (by simp)
        · exact absurd ((gv_body_at_end reads last_seq out i hlen2).symm.trans hB) (by simp)
    | div =>
      dsimp only
      constructor
      · intro h
        exact absurd h (by simp)
      · rintro (⟨out2, i2, hbody, -⟩ | ⟨hlen2, hout2⟩)
        · exact absurd (hbody.symm.trans hB) (by simp)
        · exact absurd ((gv_body_at_end reads last_seq out i hlen2).symm.trans hB) (by simp)

/-- RFC-0218 P0.1 4/4 (atom `catalog:group_validate`): a validação OCC
    do grupo inteiro é exatamente a cadeia citada do loop extraído —
    cada membro é lido (`Slice.index_usize`), decidido pelo
    `occ_conflict` extraído (atom 1/4) e empurrado no out, i cresce
    estritamente; o fim é `i = len` com o vetor de vereditos `out = v`;
    sem terceiro destino. A simultaneidade (janela vazia por membro) é o
    tooth que o AS-IS serializado perde. -/
theorem group_validate_fate_iff :
    ∀ (reads : Aeneas.Std.Slice OccRead) (last_seq : Std.U64)
      (v : alloc.vec.Vec Bool),
      (group_validate reads last_seq = ok v) ↔
        ValidateFate reads last_seq (reads.val).length
          (alloc.vec.Vec.with_capacity Bool (Aeneas.Std.Slice.len reads))
          0#usize v := by
  intro reads last_seq v
  have hloop : group_validate reads last_seq
      = group_validate_loop reads last_seq
          (alloc.vec.Vec.with_capacity Bool (Aeneas.Std.Slice.len reads))
          0#usize := by
    unfold group_validate
    rfl
  rw [hloop]
  exact group_validate_loop_fate reads last_seq (reads.val).length _ 0#usize
    (Nat.zero_le _) (Nat.sub_le _ _) v

/-- RFC-0219 P2.1 (atom `catalog:group_ack_plan`): o grupo (ou commit
    lone) acka e publica EXATAMENTE quando sua I/O de WAL teve sucesso;
    I/O falhada cerca — sem publish, sem Ok. O AS-IS acka a falha (Ok
    com mentira — tooth plantado). -/
theorem group_ack_plan_fate_iff :
    ∀ (wal_io_ok : Bool) (plan : GroupAckPlan),
      (group_ack_plan wal_io_ok = ok plan) ↔
        ((wal_io_ok = true ∧ plan = GroupAckPlan.AckPublishGroup) ∨
          (wal_io_ok = false ∧ plan = GroupAckPlan.FenceRefuseIoFail)) := by
  intro wal_io_ok plan
  unfold group_ack_plan may_publish_group
  cases wal_io_ok <;> simp_all <;> exact eq_comm

/-- Passo 2: `∀ σ, n≤2 → plan(σ) = linearization(σ)` — the rustc body
    is two `lock_alphabet_step`s. Unfolds the extracted plan. -/
theorem lock_alphabet_linearizes_n2_eq_steps :
    ∀ (a0 a1 : U8),
      lock_alphabet_linearizes_n2 a0 a1 =
        (do
          let b0 ← lock_alphabet_step false false a0
          if b0 then
            lock_alphabet_step (a0 = LOCK_ACT_ACQUIRE_WRITE)
              (a0 = LOCK_ACT_SUBMIT) a1
          else ok false) := by
  intro a0 a1
  unfold lock_alphabet_linearizes_n2
  rfl

/-- Concrete linearization: acquire-write then submit. -/
theorem lock_alphabet_write_then_submit :
    lock_alphabet_linearizes_n2 LOCK_ACT_ACQUIRE_WRITE LOCK_ACT_SUBMIT
      = ok true := by
  unfold lock_alphabet_linearizes_n2 lock_alphabet_step
    LOCK_ACT_ACQUIRE_WRITE LOCK_ACT_SUBMIT LOCK_ACT_ACQUIRE_FLUSH
    LOCK_ACT_PUBLISH
  simp

/-- Illegal order: publish then submit does not linearize. -/
theorem lock_alphabet_publish_then_submit_not :
    lock_alphabet_linearizes_n2 LOCK_ACT_PUBLISH LOCK_ACT_SUBMIT
      = ok false := by
  unfold lock_alphabet_linearizes_n2 lock_alphabet_step
    LOCK_ACT_ACQUIRE_WRITE LOCK_ACT_SUBMIT LOCK_ACT_ACQUIRE_FLUSH
    LOCK_ACT_PUBLISH
  simp

/-- AS-IS tooth: the mutant admits the illegal order. -/
theorem lock_alphabet_as_is_admits_illegal :
    lock_alphabet_linearizes_n2_as_is LOCK_ACT_PUBLISH LOCK_ACT_SUBMIT
      = ok true := by
  unfold lock_alphabet_linearizes_n2_as_is
  rfl

/-- Passo 3: the N=2 alphabet is not the OS/futex scheduler. PCT d and
    lock_interleavings_admitted stay refused. -/
theorem lock_alphabet_is_not_os_scheduler :
    ∀ (d : U64),
      forall_schedules_admitted d = ok false ∧
        lock_interleavings_admitted = ok false := by
  intro d
  constructor
  · unfold forall_schedules_admitted
    rfl
  · unfold lock_interleavings_admitted
    rfl

/-- Passo 4: admitted on the alphabet iff the pair linearizes. -/
theorem lock_alphabet_interleavings_admitted_iff :
    ∀ (a0 a1 : U8),
      lock_alphabet_interleavings_admitted a0 a1 =
        lock_alphabet_linearizes_n2 a0 a1 := by
  intro a0 a1
  unfold lock_alphabet_interleavings_admitted
  rfl

/-- RFC-0229 P1.1: harness owns the wake. -/
theorem write_group_wait_grant_harness :
    write_group_wait_grant true = ok WriteGroupWait.HarnessGrant := by
  unfold write_group_wait_grant
  rfl

/-- RFC-0229 P1.1: no PCT worker ⇒ OS park. -/
theorem write_group_wait_grant_os :
    write_group_wait_grant false = ok WriteGroupWait.OsPark := by
  unfold write_group_wait_grant
  rfl

/-- AS-IS: even a harness-owned wait is reported as OS park. -/
theorem write_group_wait_grant_as_is_os :
    write_group_wait_grant_as_is true = ok WriteGroupWait.OsPark := by
  unfold write_group_wait_grant_as_is
  rfl

/-- RFC-0229 P1.1: the grant token dual-unfolds with the N=2 alphabet. -/
theorem write_group_wait_grant_linearizes_unfolds_alphabet :
    ∀ (h : Bool),
      write_group_wait_grant_linearizes h =
        (do
          let w ← write_group_wait_grant h
          match w with
          | WriteGroupWait.HarnessGrant =>
            lock_alphabet_linearizes_n2 LOCK_ACT_ACQUIRE_WRITE LOCK_ACT_SUBMIT
          | WriteGroupWait.OsPark =>
            lock_alphabet_linearizes_n2 LOCK_ACT_ACQUIRE_WRITE LOCK_ACT_SUBMIT) := by
  intro h
  unfold write_group_wait_grant_linearizes
  rfl

/-- RFC-0229 P1.2: acquire-write, submit, publish linearizes. -/
theorem lock_alphabet_n3_write_submit_publish :
    lock_alphabet_linearizes_n3
        LOCK_ACT_ACQUIRE_WRITE LOCK_ACT_SUBMIT LOCK_ACT_PUBLISH
      = ok true := by
  unfold lock_alphabet_linearizes_n3 lock_alphabet_step
    LOCK_ACT_ACQUIRE_WRITE LOCK_ACT_SUBMIT LOCK_ACT_ACQUIRE_FLUSH
    LOCK_ACT_PUBLISH
  simp

/-- RFC-0229 P1.2: publish-before-submit is still illegal at N=3. -/
theorem lock_alphabet_n3_publish_first_not :
    lock_alphabet_linearizes_n3
        LOCK_ACT_PUBLISH LOCK_ACT_SUBMIT LOCK_ACT_ACQUIRE_WRITE
      = ok false := by
  unfold lock_alphabet_linearizes_n3 lock_alphabet_step
    LOCK_ACT_ACQUIRE_WRITE LOCK_ACT_SUBMIT LOCK_ACT_ACQUIRE_FLUSH
    LOCK_ACT_PUBLISH
  simp

/-- AS-IS tooth: illegal N=3 still linearizes. -/
theorem lock_alphabet_n3_as_is_admits_illegal :
    lock_alphabet_linearizes_n3_as_is
        LOCK_ACT_PUBLISH LOCK_ACT_SUBMIT LOCK_ACT_ACQUIRE_WRITE
      = ok true := by
  unfold lock_alphabet_linearizes_n3_as_is
  rfl

/-- N=3 is still not the OS scheduler. -/
theorem lock_alphabet_n3_not_os_forall :
    lock_interleavings_admitted = ok false ∧
      forall_schedules_admitted (4#u64) = ok false := by
  constructor
  · unfold lock_interleavings_admitted; rfl
  · unfold forall_schedules_admitted; rfl
