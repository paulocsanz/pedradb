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
