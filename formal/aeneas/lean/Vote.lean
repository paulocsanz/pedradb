-- Theorem over the Aeneas extract of production vote_kernel.rs.
-- Same statement as the Verus `ensures` / Rust `vote_decision_spec`.
import Aeneas
import VoteKernel
open Aeneas Std Result
open pedra_aeneas_vote_kernel

private theorem can_vote_ok (voted_for : Option Std.U64) (candidate_id : Std.U64) :
    ∃ b, can_vote voted_for candidate_id = ok b := by
  unfold can_vote
  cases voted_for with
  | none => exact ⟨true, rfl⟩
  | some _ => exact ⟨_, rfl⟩

private theorem log_up_to_date_ok (a b c d : Std.U64) :
    ∃ r, log_up_to_date a b c d = ok r := by
  unfold log_up_to_date
  split
  · exact ⟨true, rfl⟩
  · split
    · exact ⟨_, rfl⟩
    · exact ⟨false, rfl⟩

private theorem u64_eq_of_not_bne {x y : U64} (h : ¬ (x != y) = true) : x = y := by
  have : ¬ x ≠ y := by
    simpa [bne_iff_ne, Bool.not_eq_true] using h
  exact not_not.mp this

private theorem u64_ne_of_bne {x y : U64} (h : (x != y) = true) : x ≠ y := by
  simpa [bne_iff_ne] using h

/-- Kernel outcome matches the closed-form spec, for all inputs. -/
theorem vote_decision_matches_spec (i : VoteInputs) :
    (do
      let d ← vote_decision i
      vote_decision_spec i d) = ok true := by
  unfold vote_decision
  split
  · have hne' := u64_ne_of_bne ‹_›
    simp [vote_decision_spec, hne']
  · have heq' := u64_eq_of_not_bne ‹_›
    obtain ⟨can, hcan⟩ := can_vote_ok i.voted_for i.candidate_id
    rw [hcan]
    simp only [bind_tc_ok]
    split
    · obtain ⟨up, hup⟩ :=
        log_up_to_date_ok i.last_log_term i.last_log_index
          i.candidate_last_log_term i.candidate_last_log_index
      rw [hup]
      simp only [bind_tc_ok]
      split <;> simp_all [vote_decision_spec]
    · simp_all [vote_decision_spec]

/-- RFC-0053 P40: iff on the **extracted** term `VoteKernel.vote_decision`
    (not the Verus twin). Same closed form as the Verus `ensures`. -/
theorem vote_decision_iff (i : VoteInputs) :
    vote_decision i = ok .WouldGrant ↔
      i.candidate_term = i.current_term ∧
      can_vote i.voted_for i.candidate_id = ok true ∧
      log_up_to_date i.last_log_term i.last_log_index
          i.candidate_last_log_term i.candidate_last_log_index = ok true := by
  constructor
  · intro h
    unfold vote_decision at h
    split at h
    · simp at h
    · have heq' := u64_eq_of_not_bne ‹_›
      obtain ⟨can, hcan⟩ := can_vote_ok i.voted_for i.candidate_id
      rw [hcan] at h
      simp only [bind_tc_ok] at h
      split at h
      · obtain ⟨up, hup⟩ :=
          log_up_to_date_ok i.last_log_term i.last_log_index
            i.candidate_last_log_term i.candidate_last_log_index
        rw [hup] at h
        simp only [bind_tc_ok] at h
        split at h
        · have hc : can = true := ‹can = true›
          have hu : up = true := ‹up = true›
          rw [hc] at hcan
          rw [hu] at hup
          exact ⟨heq', hcan, hup⟩
        · simp at h
      · simp at h
  · intro ⟨heq, hcan, hup⟩
    unfold vote_decision
    split
    · have hne' := u64_ne_of_bne ‹_›
      exact (hne' heq).elim
    · rw [hcan]
      simp only [bind_tc_ok]
      split
      · rw [hup]
        simp
      · simp_all

theorem vote_decision_total (i : VoteInputs) : ∃ d, vote_decision i = ok d := by
  have h := vote_decision_matches_spec i
  cases hdd : vote_decision i with
  | ok d => exact ⟨d, rfl⟩
  | fail e => rw [hdd] at h; exact absurd h (by simp)
  | div => rw [hdd] at h; exact absurd h (by simp)

/-- RFC-0205 P0.2 (registered atom): the vote fate over ALL inputs —
    WouldGrant exactly on the conjunction same-term ∧ can_vote ∧
    log_up_to_date; Deny on its negation (two-constructor decision;
    totality from the spec match). The P40 grant-iff above pins the
    grant side; this adds the Deny side and the ∀ form the registry
    gate requires. -/
theorem vote_decision_fate_iff :
    ∀ (i : VoteInputs) (d : VoteDecision),
      (vote_decision i = ok d) ↔
        ((d = VoteDecision.WouldGrant ∧
            i.candidate_term = i.current_term ∧
            can_vote i.voted_for i.candidate_id = ok true ∧
            log_up_to_date i.last_log_term i.last_log_index
                i.candidate_last_log_term i.candidate_last_log_index = ok true) ∨
         (d = VoteDecision.Deny ∧
            ¬ (i.candidate_term = i.current_term ∧
            can_vote i.voted_for i.candidate_id = ok true ∧
            log_up_to_date i.last_log_term i.last_log_index
                i.candidate_last_log_term i.candidate_last_log_index = ok true))) := by
  intro i d
  have hgrant := vote_decision_iff i
  have htotal := vote_decision_total i
  cases d with
  | WouldGrant =>
    constructor
    · intro hval
      exact Or.inl ⟨rfl, hgrant.1 hval⟩
    · rintro (⟨_, hconds⟩ | ⟨hne, _⟩)
      · exact hgrant.2 hconds
      · simp at hne
  | Deny =>
    constructor
    · intro hval
      refine Or.inr ⟨rfl, ?_⟩
      intro hconds
      have hw := hgrant.2 hconds
      rw [hw] at hval
      exact absurd hval (by simp)
    · rintro (⟨hw, _⟩ | ⟨_, hneg⟩)
      · simp at hw
      · have hnotgrant : ¬ (vote_decision i = ok .WouldGrant) :=
          fun hw => hneg (hgrant.1 hw)
        obtain ⟨d', hd'⟩ := htotal
        cases d' with
        | WouldGrant => exact absurd hd' hnotgrant
        | Deny => exact hd'

private abbrev staleOther : VoteInputs := {
  current_term := 5#u64
  voted_for := some (2#u64)
  last_log_term := 3#u64
  last_log_index := 10#u64
  candidate_term := 5#u64
  candidate_id := 1#u64
  candidate_last_log_term := 3#u64
  candidate_last_log_index := 10#u64
}

/-- F15 refinement: a wire grant implies persist Ok. Disk remaining an axiom
    is exactly this implication — we never claim persist itself. -/
theorem grant_after_persist_implies_ok
    (d : VoteDecision) (p : PersistOutcome) :
    grant_after_persist d p = ok true → p = .Ok := by
  cases d <;> cases p <;> intro h
  · rfl
  · simp [grant_after_persist] at h
  · simp [grant_after_persist] at h
  · simp [grant_after_persist] at h

/-- The AS-IS mutant grants a candidate the follower already voted against
    (F15 teeth — the invariant is not vacuous). -/
theorem as_is_grants_where_fixed_denies :
    vote_decision staleOther = ok .Deny
    ∧ vote_decision_as_is_ignore_log_and_vote staleOther = ok .WouldGrant := by
  constructor
  · unfold vote_decision can_vote
    simp
  · unfold vote_decision_as_is_ignore_log_and_vote
    simp

/-- Catalog entry: the term is durably raised exactly when the incoming
    term is strictly newer AND the persist succeeded (RFC-0158 — a
    failed persist restores the old term, never keeps a raised one). -/
theorem durable_term_if_newer_raised_iff_newer_and_persisted :
    ∀ (current_term incoming_term : Std.U64) (persist : PersistOutcome),
      (durable_term_if_newer current_term incoming_term persist
          = ok DurableTerm.Raised)
        ↔ ((incoming_term > current_term)
            ∧ (persist = PersistOutcome.Ok)) := by
  intro current_term incoming_term persist
  unfold durable_term_if_newer
  constructor
  · intro h
    split at h
    · next c1 =>
      split at h
      · next c2 => exact ⟨c1, rfl⟩
      · next c2 => exact absurd h (by simp)
    · next c1 => exact absurd h (by simp)
  · rintro ⟨ht, hp⟩
    rw [if_pos ht]
    split
    · rfl
    · exact absurd hp (by simp)
/-- RFC-0208 P0.1 (seam raft 1/2, atom `catalog:grant_persist`): the
    vote is granted EXACTLY when the decision is WouldGrant AND the
    durable persist succeeded — Deny never grants, and a failed persist
    of a WouldGrant never grants either (the election path never
    hands out a vote it did not make durable). -/
theorem grant_after_persist_fate_iff :
    ∀ (decision : VoteDecision) (persist : PersistOutcome) (v : Bool),
      (grant_after_persist decision persist = ok v) ↔
        ((v = true ∧ decision = VoteDecision.WouldGrant
            ∧ persist = PersistOutcome.Ok)
          ∨ (v = false ∧ ¬(decision = VoteDecision.WouldGrant
            ∧ persist = PersistOutcome.Ok))) := by
  intro decision persist v
  cases decision with
  | WouldGrant =>
    cases persist with
    | Ok => cases v <;> simp [grant_after_persist]
    | Err => cases v <;> simp [grant_after_persist]
  | Deny => cases v <;> simp [grant_after_persist]
