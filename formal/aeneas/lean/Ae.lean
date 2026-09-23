-- Theorems over the Aeneas extract of production ae_kernel.rs (F16).
-- RFC-0053 P2.1: second machine (not the Verus twin).
import Aeneas
import AeKernel
open Aeneas Std Result
open pedra_aeneas_ae_kernel

/-- Same-term existing entry is Keep. -/
theorem ae_keep_if_same_term (idx term commit last : U64) :
    ae_entry_action idx term (some term) commit last = ok .Keep := by
  unfold ae_entry_action
  simp

/-- Same-term existing entry is Keep (production unit-test point). -/
theorem ae_keep_same_index_term :
    ae_entry_action (2#u64) (7#u64) (some (7#u64)) (1#u64) (5#u64)
      = ok .Keep :=
  ae_keep_if_same_term (2#u64) (7#u64) (1#u64) (5#u64)

/-- F16: conflict at commit index is Refuse, not truncate. -/
theorem ae_refuse_conflict_at_commit :
    ae_entry_action (1#u64) (9#u64) (some (3#u64)) (1#u64) (5#u64)
      = ok .Refuse := by
  unfold ae_entry_action
  simp

/-- Uncommitted term conflict truncates. -/
theorem ae_truncate_conflict_after_commit :
    ae_entry_action (3#u64) (9#u64) (some (3#u64)) (1#u64) (5#u64)
      = ok .TruncateAndInstall := by
  unfold ae_entry_action
  simp

/-- F16 teeth: AS-IS mutant rewrites a committed slot. -/
theorem as_is_rewrites_committed :
    ae_entry_action (1#u64) (9#u64) (some (3#u64)) (1#u64) (5#u64)
        = ok .Refuse ∧
      ae_entry_action_as_is_rewrite_committed
          (1#u64) (9#u64) (some (3#u64)) (1#u64) (5#u64)
        = ok .TruncateAndInstall := by
  constructor
  · exact ae_refuse_conflict_at_commit
  · unfold ae_entry_action_as_is_rewrite_committed
    simp

/-- Catalog entry: an AppendEntries ack succeeds exactly when the log
    is clean, or it is dirty and the persist succeeded (F48 — a dirty
    log whose persist failed never acks ok). -/
theorem ae_ack_success_ok_iff_clean_or_dirty_persisted :
    ∀ (log_dirty : Bool) (persist_ok : Bool),
      (ae_ack_success log_dirty persist_ok = ok true)
        ↔ (log_dirty = false ∨ persist_ok = true) := by
  intro log_dirty persist_ok
  unfold ae_ack_success
  constructor
  · intro h
    split at h
    · next c1 =>
      simp at h
      exact Or.inr h
    · next c1 => exact Or.inl (by simp at c1; exact c1)
  · rintro (h1 | hc)
    · rw [if_neg (by simp [h1])]
    · split
      · next _ => rw [hc]
      · rfl

/-- Catalog entry: a conflicting entry truncates and reinstalls exactly
    when a different term already sits at that slot AND the slot is
    above the commit index — a conflict at or below commit is refused,
    never rewritten (F16). -/
theorem ae_entry_action_truncate_and_install_iff_conflict_above_commit :
    ∀ (entry_index : U64) (entry_term : U64) (existing_term : Option U64)
      (commit_index : U64) (last_log_index : U64),
      (ae_entry_action entry_index entry_term existing_term commit_index last_log_index
          = ok AeEntryAction.TruncateAndInstall)
        ↔ (∃ t, existing_term = some t ∧ ¬(t = entry_term)
            ∧ ¬(entry_index <= commit_index)) := by
  intro entry_index entry_term existing_term commit_index last_log_index
  unfold ae_entry_action
  cases existing_term with
  | none =>
    constructor
    · intro h
      simp at h
      have hl : lift (core.num.U64.saturating_add last_log_index 1#u64)
          = ok (core.num.U64.saturating_add last_log_index 1#u64) := rfl
      rw [hl] at h
      simp at h
      split at h
      · exact absurd h (by simp)
      · exact absurd h (by simp)
    · rintro ⟨t, habsurd, _, _⟩
      exact absurd habsurd (by simp)
  | some t =>
    constructor
    · intro h
      simp at h
      split at h
      · next hkeep =>
        exact absurd h (by simp)
      · next hne =>
        split at h
        · next hle =>
          exact absurd h (by simp)
        · next hgt =>
          exact ⟨t, rfl, hne, hgt⟩
    · rintro ⟨t', hex, hne, hgt⟩
      simp only [Option.some.injEq] at hex
      subst hex
      simp [hne, hgt]

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

/-- RFC-0218 P2.2 (átomo `catalog:ae_f16_gate`, entrada `ae_f16_safe`):
    o gate F16 é exatamente a árvore citada — truncar só passa acima do
    commit; append só na ponta esperada (saturating_add) com slot vazio;
    conflito decide pelo match do Refuse, nunca reescreve slot commitado.
    O AS-IS reescreve slot commitado (dente plantado). -/

private theorem ae_refuse_match_ok (action : AeEntryAction) :
    ∃ br : Bool,
      (match action with
       | AeEntryAction.Keep => ok false
       | AeEntryAction.Append => ok false
       | AeEntryAction.TruncateAndInstall => ok false
       | AeEntryAction.Refuse => ok true) = ok br := by
  cases action <;> exact ⟨_, rfl⟩

theorem ae_f16_safe_fate_iff :
    ∀ (entry_index entry_term : U64) (existing_term : Option U64)
      (commit_index last_log_index : U64) (action : AeEntryAction)
      (v : Bool),
      (ae_f16_safe entry_index entry_term existing_term commit_index
          last_log_index action = ok v) ↔
        (∃ b : Bool,
          (match action with
           | AeEntryAction.Keep => ok false
           | AeEntryAction.Append => ok false
           | AeEntryAction.TruncateAndInstall => ok true
           | AeEntryAction.Refuse => ok false) = ok b ∧
        ∃ br : Bool,
          (match action with
           | AeEntryAction.Keep => ok false
           | AeEntryAction.Append => ok false
           | AeEntryAction.TruncateAndInstall => ok false
           | AeEntryAction.Refuse => ok true) = ok br ∧
        (if b = true then
           (if entry_index <= commit_index then v = false
            else
              ∃ b1 : Bool,
                (match action with
                 | AeEntryAction.Keep => ok false
                 | AeEntryAction.Append => ok true
                 | AeEntryAction.TruncateAndInstall => ok false
                 | AeEntryAction.Refuse => ok false) = ok b1 ∧
                (if b1 = true then
                   (if core.option.Option.is_some existing_term = true then
                      v = false
                    else
                      ∃ i : U64,
                        lift (core.num.U64.saturating_add last_log_index
                            1#u64) = ok i ∧
                        (if entry_index != i then v = false
                         else
                           match existing_term with
                           | none => v = true
                           | some t =>
                               if t != entry_term then
                                 (if entry_index <= commit_index then
                                    (if br = true then v = true
                                     else v = false)
                                 else v = true)
                               else v = true))
                 else
                   match existing_term with
                   | none => v = true
                   | some t =>
                       if t != entry_term then
                         (if entry_index <= commit_index then
                            (if br = true then v = true else v = false)
                          else v = true)
                       else v = true))
         else
           ∃ b1 : Bool,
             (match action with
              | AeEntryAction.Keep => ok false
              | AeEntryAction.Append => ok true
              | AeEntryAction.TruncateAndInstall => ok false
              | AeEntryAction.Refuse => ok false) = ok b1 ∧
             (if b1 = true then
                (if core.option.Option.is_some existing_term = true then
                   v = false
                 else
                   ∃ i : U64,
                     lift (core.num.U64.saturating_add last_log_index
                         1#u64) = ok i ∧
                     (if entry_index != i then v = false
                      else
                        match existing_term with
                        | none => v = true
                        | some t =>
                            if t != entry_term then
                              (if entry_index <= commit_index then
                                 (if br = true then v = true
                                  else v = false)
                               else v = true)
                            else v = true))
              else
                match existing_term with
                | none => v = true
                | some t =>
                    if t != entry_term then
                      (if entry_index <= commit_index then
                         (if br = true then v = true else v = false)
                       else v = true)
                    else v = true))) := by
  intro entry_index entry_term existing_term commit_index last_log_index action v
  cases existing_term with
  | none =>
    constructor
    · intro hval
      unfold ae_f16_safe at hval
      obtain ⟨b, hb, hval⟩ := bind_ok_inv _ _ _ hval
      obtain ⟨br, hbr⟩ := ae_refuse_match_ok action
      refine ⟨b, hb, br, hbr, ?_⟩
      cases b with
      | true =>
        rw [if_pos (by simp : (true : Bool) = true)]
        rw [if_pos (by simp : (true : Bool) = true)] at hval
        by_cases hle : entry_index ≤ commit_index
        · rw [if_pos hle]
          rw [if_pos hle] at hval
          injection hval with hv
          exact hv.symm
        · rw [if_neg hle]
          rw [if_neg hle] at hval
          obtain ⟨b1, hb1, hval⟩ := bind_ok_inv _ _ _ hval
          refine ⟨b1, hb1, ?_⟩
          cases b1 with
          | true =>
            rw [if_pos (by simp : (true : Bool) = true)]
            rw [if_pos (by simp : (true : Bool) = true)] at hval
            first | dsimp only at hval | skip
            rw [if_neg (by simp : ¬(core.option.Option.is_some (none : Option U64) = true))]
            rw [if_neg (by simp : ¬(core.option.Option.is_some (none : Option U64) = true))] at hval
            obtain ⟨i, hi, hval⟩ := bind_ok_inv _ _ _ hval
            refine ⟨i, hi, ?_⟩
            by_cases hne : (entry_index != i) = true
            · rw [if_pos hne]
              rw [if_pos hne] at hval
              injection hval with hv
              exact hv.symm
            · rw [if_neg hne]
              rw [if_neg hne] at hval
              first | dsimp only | skip
              first | dsimp only at hval | skip
              injection hval with hv
              exact hv.symm
          | false =>
            rw [if_neg (by simp : ¬((false : Bool) = true))]
            rw [if_neg (by simp : ¬((false : Bool) = true))] at hval
            first | dsimp only | skip
            first | dsimp only at hval | skip
            injection hval with hv
            exact hv.symm
      | false =>
        rw [if_neg (by simp : ¬((false : Bool) = true))]
        rw [if_neg (by simp : ¬((false : Bool) = true))] at hval
        obtain ⟨b1, hb1, hval⟩ := bind_ok_inv _ _ _ hval
        refine ⟨b1, hb1, ?_⟩
        cases b1 with
        | true =>
          rw [if_pos (by simp : (true : Bool) = true)]
          rw [if_pos (by simp : (true : Bool) = true)] at hval
          first | dsimp only at hval | skip
          rw [if_neg (by simp : ¬(core.option.Option.is_some (none : Option U64) = true))]
          rw [if_neg (by simp : ¬(core.option.Option.is_some (none : Option U64) = true))] at hval
          obtain ⟨i, hi, hval⟩ := bind_ok_inv _ _ _ hval
          refine ⟨i, hi, ?_⟩
          by_cases hne : (entry_index != i) = true
          · rw [if_pos hne]
            rw [if_pos hne] at hval
            injection hval with hv
            exact hv.symm
          · rw [if_neg hne]
            rw [if_neg hne] at hval
            first | dsimp only | skip
            first | dsimp only at hval | skip
            injection hval with hv
            exact hv.symm
        | false =>
          rw [if_neg (by simp : ¬((false : Bool) = true))]
          rw [if_neg (by simp : ¬((false : Bool) = true))] at hval
          first | dsimp only | skip
          first | dsimp only at hval | skip
          injection hval with hv
          exact hv.symm
    · rintro ⟨b, hb, br, hbr, htree⟩
      unfold ae_f16_safe
      refine bind_intro b hb ?_
      first | dsimp only | skip
      cases b with
      | true =>
        rw [if_pos (by simp : (true : Bool) = true)]
        rw [if_pos (by simp : (true : Bool) = true)] at htree
        by_cases hle : entry_index ≤ commit_index
        · rw [if_pos hle]
          rw [if_pos hle] at htree
          rw [htree]
        · rw [if_neg hle]
          rw [if_neg hle] at htree
          obtain ⟨b1, hb1, htree⟩ := htree
          refine bind_intro b1 hb1 ?_
          first | dsimp only | skip
          cases b1 with
          | true =>
            rw [if_pos (by simp : (true : Bool) = true)]
            rw [if_pos (by simp : (true : Bool) = true)] at htree
            first | dsimp only | skip
            first | dsimp only at htree | skip
            rw [if_neg (by simp : ¬(core.option.Option.is_some (none : Option U64) = true))]
            rw [if_neg (by simp : ¬(core.option.Option.is_some (none : Option U64) = true))] at htree
            obtain ⟨i, hi, htree⟩ := htree
            refine bind_intro i hi ?_
            first | dsimp only | skip
            by_cases hne : (entry_index != i) = true
            · rw [if_pos hne]
              rw [if_pos hne] at htree
              rw [htree]
            · rw [if_neg hne]
              rw [if_neg hne] at htree
              first | dsimp only | skip
              first | dsimp only at htree | skip
              rw [htree]
          | false =>
            rw [if_neg (by simp : ¬((false : Bool) = true))]
            rw [if_neg (by simp : ¬((false : Bool) = true))] at htree
            first | dsimp only | skip
            first | dsimp only at htree | skip
            rw [htree]
      | false =>
        rw [if_neg (by simp : ¬((false : Bool) = true))]
        rw [if_neg (by simp : ¬((false : Bool) = true))] at htree
        obtain ⟨b1, hb1, htree⟩ := htree
        refine bind_intro b1 hb1 ?_
        first | dsimp only | skip
        cases b1 with
        | true =>
          rw [if_pos (by simp : (true : Bool) = true)]
          rw [if_pos (by simp : (true : Bool) = true)] at htree
          first | dsimp only | skip
          first | dsimp only at htree | skip
          rw [if_neg (by simp : ¬(core.option.Option.is_some (none : Option U64) = true))]
          rw [if_neg (by simp : ¬(core.option.Option.is_some (none : Option U64) = true))] at htree
          obtain ⟨i, hi, htree⟩ := htree
          refine bind_intro i hi ?_
          first | dsimp only | skip
          by_cases hne : (entry_index != i) = true
          · rw [if_pos hne]
            rw [if_pos hne] at htree
            rw [htree]
          · rw [if_neg hne]
            rw [if_neg hne] at htree
            first | dsimp only | skip
            first | dsimp only at htree | skip
            rw [htree]
        | false =>
          rw [if_neg (by simp : ¬((false : Bool) = true))]
          rw [if_neg (by simp : ¬((false : Bool) = true))] at htree
          first | dsimp only | skip
          first | dsimp only at htree | skip
          rw [htree]
  | some t =>
    constructor
    · intro hval
      unfold ae_f16_safe at hval
      obtain ⟨b, hb, hval⟩ := bind_ok_inv _ _ _ hval
      obtain ⟨br, hbr⟩ := ae_refuse_match_ok action
      refine ⟨b, hb, br, hbr, ?_⟩
      cases b with
      | true =>
        rw [if_pos (by simp : (true : Bool) = true)]
        rw [if_pos (by simp : (true : Bool) = true)] at hval
        by_cases hle : entry_index ≤ commit_index
        · rw [if_pos hle]
          rw [if_pos hle] at hval
          injection hval with hv
          exact hv.symm
        · rw [if_neg hle]
          rw [if_neg hle] at hval
          obtain ⟨b1, hb1, hval⟩ := bind_ok_inv _ _ _ hval
          refine ⟨b1, hb1, ?_⟩
          cases b1 with
          | true =>
            rw [if_pos (by simp : (true : Bool) = true)]
            rw [if_pos (by simp : (true : Bool) = true)] at hval
            first | dsimp only at hval | skip
            rw [if_pos (by simp : core.option.Option.is_some (some t) = true)]
            rw [if_pos (by simp : core.option.Option.is_some (some t) = true)] at hval
            injection hval with hv
            exact hv.symm
          | false =>
            rw [if_neg (by simp : ¬((false : Bool) = true))]
            rw [if_neg (by simp : ¬((false : Bool) = true))] at hval
            first | dsimp only | skip
            first | dsimp only at hval | skip
            by_cases ht : (t != entry_term) = true
            · rw [if_pos ht]
              rw [if_pos ht] at hval
              by_cases hle2 : entry_index ≤ commit_index
              · rw [if_pos hle2]
                rw [if_pos hle2] at hval
                cases br with
                | true =>
                    injection hbr.symm.trans hval with hv
                    exact hv.symm
                | false =>
                    injection hbr.symm.trans hval with hv
                    exact hv.symm
              · rw [if_neg hle2]
                rw [if_neg hle2] at hval
                injection hval with hv
                exact hv.symm
            · rw [if_neg ht]
              rw [if_neg ht] at hval
              injection hval with hv
              exact hv.symm
      | false =>
        rw [if_neg (by simp : ¬((false : Bool) = true))]
        rw [if_neg (by simp : ¬((false : Bool) = true))] at hval
        obtain ⟨b1, hb1, hval⟩ := bind_ok_inv _ _ _ hval
        refine ⟨b1, hb1, ?_⟩
        cases b1 with
        | true =>
          rw [if_pos (by simp : (true : Bool) = true)]
          rw [if_pos (by simp : (true : Bool) = true)] at hval
          first | dsimp only at hval | skip
          rw [if_pos (by simp : core.option.Option.is_some (some t) = true)]
          rw [if_pos (by simp : core.option.Option.is_some (some t) = true)] at hval
          injection hval with hv
          exact hv.symm
        | false =>
          rw [if_neg (by simp : ¬((false : Bool) = true))]
          rw [if_neg (by simp : ¬((false : Bool) = true))] at hval
          first | dsimp only | skip
          first | dsimp only at hval | skip
          by_cases ht : (t != entry_term) = true
          · rw [if_pos ht]
            rw [if_pos ht] at hval
            by_cases hle2 : entry_index ≤ commit_index
            · rw [if_pos hle2]
              rw [if_pos hle2] at hval
              cases br with
              | true =>
                  injection hbr.symm.trans hval with hv
                  exact hv.symm
              | false =>
                  injection hbr.symm.trans hval with hv
                  exact hv.symm
            · rw [if_neg hle2]
              rw [if_neg hle2] at hval
              injection hval with hv
              exact hv.symm
          · rw [if_neg ht]
            rw [if_neg ht] at hval
            injection hval with hv
            exact hv.symm
    · rintro ⟨b, hb, br, hbr, htree⟩
      unfold ae_f16_safe
      refine bind_intro b hb ?_
      first | dsimp only | skip
      cases b with
      | true =>
        rw [if_pos (by simp : (true : Bool) = true)]
        rw [if_pos (by simp : (true : Bool) = true)] at htree
        by_cases hle : entry_index ≤ commit_index
        · rw [if_pos hle]
          rw [if_pos hle] at htree
          rw [htree]
        · rw [if_neg hle]
          rw [if_neg hle] at htree
          obtain ⟨b1, hb1, htree⟩ := htree
          refine bind_intro b1 hb1 ?_
          first | dsimp only | skip
          cases b1 with
          | true =>
            rw [if_pos (by simp : (true : Bool) = true)]
            rw [if_pos (by simp : (true : Bool) = true)] at htree
            first | dsimp only | skip
            first | dsimp only at htree | skip
            rw [if_pos (by simp : core.option.Option.is_some (some t) = true)]
            rw [if_pos (by simp : core.option.Option.is_some (some t) = true)] at htree
            rw [htree]
          | false =>
            rw [if_neg (by simp : ¬((false : Bool) = true))]
            rw [if_neg (by simp : ¬((false : Bool) = true))] at htree
            first | dsimp only | skip
            first | dsimp only at htree | skip
            by_cases ht : (t != entry_term) = true
            · rw [if_pos ht]
              rw [if_pos ht] at htree
              by_cases hle2 : entry_index ≤ commit_index
              · rw [if_pos hle2]
                rw [if_pos hle2] at htree
                cases br with
                | true =>
                    rw [if_pos (by simp : (true : Bool) = true)] at htree
                    exact hbr.trans (congrArg ok htree.symm)
                | false =>
                    rw [if_neg (by simp : ¬((false : Bool) = true))] at htree
                    exact hbr.trans (congrArg ok htree.symm)
              · rw [if_neg hle2]
                rw [if_neg hle2] at htree
                rw [htree]
            · rw [if_neg ht]
              rw [if_neg ht] at htree
              rw [htree]
      | false =>
        rw [if_neg (by simp : ¬((false : Bool) = true))]
        rw [if_neg (by simp : ¬((false : Bool) = true))] at htree
        obtain ⟨b1, hb1, htree⟩ := htree
        refine bind_intro b1 hb1 ?_
        first | dsimp only | skip
        cases b1 with
        | true =>
          rw [if_pos (by simp : (true : Bool) = true)]
          rw [if_pos (by simp : (true : Bool) = true)] at htree
          first | dsimp only | skip
          first | dsimp only at htree | skip
          rw [if_pos (by simp : core.option.Option.is_some (some t) = true)]
          rw [if_pos (by simp : core.option.Option.is_some (some t) = true)] at htree
          rw [htree]
        | false =>
          rw [if_neg (by simp : ¬((false : Bool) = true))]
          rw [if_neg (by simp : ¬((false : Bool) = true))] at htree
          first | dsimp only | skip
          first | dsimp only at htree | skip
          by_cases ht : (t != entry_term) = true
          · rw [if_pos ht]
            rw [if_pos ht] at htree
            by_cases hle2 : entry_index ≤ commit_index
            · rw [if_pos hle2]
              rw [if_pos hle2] at htree
              cases br with
              | true =>
                  rw [if_pos (by simp : (true : Bool) = true)] at htree
                  exact hbr.trans (congrArg ok htree.symm)
              | false =>
                  rw [if_neg (by simp : ¬((false : Bool) = true))] at htree
                  exact hbr.trans (congrArg ok htree.symm)
            · rw [if_neg hle2]
              rw [if_neg hle2] at htree
              rw [htree]
          · rw [if_neg ht]
            rw [if_neg ht] at htree
            rw [htree]
