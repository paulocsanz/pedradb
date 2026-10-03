-- Theorems over Aeneas extract of store vote_kernel.rs (F15).
import Aeneas
import StoreVoteKernel
open Aeneas.Std Result
open pedra_aeneas_store_vote_kernel

/-- Stale candidate term is Deny. -/
theorem vote_decision_stale_term :
    vote_decision
      { current_term := 2#u64
        voted_for := none
        last_log_term := 1#u64
        last_log_index := 1#u64
        candidate_term := 1#u64
        candidate_id := 7#u64
        candidate_last_log_term := 1#u64
        candidate_last_log_index := 1#u64 } = ok VoteDecision.Deny := by
  unfold vote_decision
  rfl

/-- AS-IS dente: same-term grant ignores log and prior vote. -/
theorem vote_decision_as_is_dente :
    vote_decision_as_is_ignore_log_and_vote
      { current_term := 1#u64
        voted_for := some (3#u64)
        last_log_term := 1#u64
        last_log_index := 9#u64
        candidate_term := 1#u64
        candidate_id := 7#u64
        candidate_last_log_term := 0#u64
        candidate_last_log_index := 0#u64 } = ok VoteDecision.WouldGrant := by
  unfold vote_decision_as_is_ignore_log_and_vote
  rfl
