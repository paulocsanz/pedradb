-- Cross-lib: the store/raft seam (RFC-0208 P1.1) — the election
-- chain and the recovery triple COMPOSED over registered atoms.
-- Registration rule: a row needs a single catalog pair/entry; these
-- compositions span several kernels (vote × vote, membership ×
-- membership) — same reason the other compose libs carry no row.
import Aeneas
import Vote
import Commit
import Membership
open Aeneas.Std Result
open pedra_aeneas_vote_kernel
open pedra_aeneas_commit_kernel
open pedra_aeneas_membership_kernel

/-- An ok-valued Result bind forces the bound term to be ok. -/
private theorem bind_ok_inv {α β} (x : Result α) (f : α → Result β) (v : β)
    (h : Aeneas.Std.bind x f = ok v) : ∃ a, x = ok a ∧ f a = ok v := by
  cases x with
  | ok a => exact ⟨a, rfl, h⟩
  | fail e => exact absurd h (by simp)
  | div => exact absurd h (by simp)

/-- An ok chain reassembles into an ok bind. -/
private theorem bind_intro {α β} {x : Result α} {f : α → Result β} {v : β}
    (a : α) (hx : x = ok a) (h : f a = ok v) : Aeneas.Std.bind x f = ok v := by
  rw [hx]; exact h

/-! ### The election chain (registered atoms `catalog:vote` ×
`catalog:grant_persist`) -/

/-- RFC-0208 P1.1: the ELECTION CHAIN composed over the two
    registered vote atoms — `vote_decision` decides (atom
    `catalog:vote`), the durable persist gates the concession (atom
    `catalog:grant_persist`). The grant that leaves the handler is
    true EXACTLY when the decision conditions hold AND the persist
    succeeded: a Deny never grants, and a failed persist of a
    WouldGrant never grants either. -/
theorem election_grant_chain_fate :
    ∀ (i : VoteInputs) (persist : PersistOutcome) (v : Bool),
      (Aeneas.Std.bind (vote_decision i)
          (fun d => grant_after_persist d persist) = ok v) ↔
        ((v = true ∧
            i.candidate_term = i.current_term ∧
            can_vote i.voted_for i.candidate_id = ok true ∧
            log_up_to_date i.last_log_term i.last_log_index
                i.candidate_last_log_term i.candidate_last_log_index
              = ok true ∧
            persist = PersistOutcome.Ok)
          ∨ (v = false ∧
            ¬ (i.candidate_term = i.current_term ∧
            can_vote i.voted_for i.candidate_id = ok true ∧
            log_up_to_date i.last_log_term i.last_log_index
                i.candidate_last_log_term i.candidate_last_log_index
              = ok true ∧
            persist = PersistOutcome.Ok))) := by
  intro i persist v
  constructor
  · intro hval
    obtain ⟨d, hw, hm⟩ := bind_ok_inv _ _ _ hval
    rcases (vote_decision_fate_iff i d).mp hw with
      ⟨hd, ht, hc, hl⟩ | ⟨hd, hne⟩
    · rcases (grant_after_persist_fate_iff d persist v).mp hm with
        ⟨rfl, _, hp⟩ | ⟨rfl, hnp⟩
      · exact Or.inl ⟨rfl, ht, hc, hl, hp⟩
      · exact Or.inr ⟨rfl, fun h => hnp ⟨hd, h.2.2.2⟩⟩
    · rcases (grant_after_persist_fate_iff d persist v).mp hm with
        ⟨rfl, hneq, _⟩ | ⟨rfl, _⟩
      · rw [hd] at hneq; exact absurd hneq (by simp)
      · exact Or.inr ⟨rfl, fun h => hne ⟨h.1, h.2.1, h.2.2.1⟩⟩
  · rintro (⟨rfl, ht, hc, hl, hp⟩ | ⟨rfl, hne⟩)
    · refine bind_intro _ ((vote_decision_fate_iff i
          VoteDecision.WouldGrant).mpr (Or.inl ⟨rfl, ht, hc, hl⟩)) ?_
      exact (grant_after_persist_fate_iff _ _ _).mpr (Or.inl ⟨rfl, rfl, hp⟩)
    · cases persist with
      | Ok =>
        have hne' : ¬ (i.candidate_term = i.current_term ∧
            can_vote i.voted_for i.candidate_id = ok true ∧
            log_up_to_date i.last_log_term i.last_log_index
                i.candidate_last_log_term i.candidate_last_log_index
              = ok true) := fun h => hne ⟨h.1, h.2.1, h.2.2, rfl⟩
        refine bind_intro _ ((vote_decision_fate_iff i
            VoteDecision.Deny).mpr (Or.inr ⟨rfl, hne'⟩)) ?_
        exact (grant_after_persist_fate_iff _ _ _).mpr (Or.inr ⟨rfl, by simp⟩)
      | Err =>
        obtain ⟨d, hd⟩ := vote_decision_total i
        refine bind_intro d hd ?_
        exact (grant_after_persist_fate_iff d _ _).mpr (Or.inr ⟨rfl, by simp⟩)

/-! ### The recovery triple (registered atoms `catalog:recover_apply`
× `catalog:recover_drop_orphan` + the extracted node-counts body) -/

/-- RFC-0208 P1.1: the RECOVERY TRIPLE composed — re-apply exactly
    when commit > applied, drop the orphan segment exactly when
    seg_index > new_hi, and the node-counts gate passes exactly on a
    local node. Each conjunct is the registered atom specialized;
    the triple is the seam a recovery pass walks. -/
theorem recovery_fate_composed :
    ∀ (applied commit seg new_hi : U64) (is_local in_ids : Bool),
      ((recover_must_apply applied commit = ok true) ↔ (commit > applied))
      ∧ ((recover_drop_orphan_seg seg new_hi = ok true) ↔ (seg > new_hi))
      ∧ ((recover_apply_node_counts is_local in_ids = ok true)
          ↔ (is_local = true)) := by
  intro applied commit seg new_hi is_local in_ids
  refine ⟨?_, ?_, ?_⟩
  · constructor
    · intro h
      rcases (recover_must_apply_fate_iff applied commit true).mp h with
        ⟨_, hP⟩ | ⟨hfalse, _⟩
      · exact hP
      · exact absurd hfalse (by simp)
    · intro hP
      exact (recover_must_apply_fate_iff applied commit true).mpr
        (Or.inl ⟨rfl, hP⟩)
  · constructor
    · intro h
      rcases (recover_drop_orphan_seg_fate_iff seg new_hi true).mp h with
        ⟨_, hP⟩ | ⟨hfalse, _⟩
      · exact hP
      · exact absurd hfalse (by simp)
    · intro hP
      exact (recover_drop_orphan_seg_fate_iff seg new_hi true).mpr
        (Or.inl ⟨rfl, hP⟩)
  · constructor
    · intro h
      simp [recover_apply_node_counts] at h
      exact h
    · intro h
      simp [recover_apply_node_counts, h]
