-- Theorems over Aeneas extract of raft membership_kernel.rs (Raft §6).
-- elect_claim_banner &'static str bottoms patched to toStr in aeneas_membership.sh.
import Aeneas
import MembershipKernel
open Aeneas.Std Result
open pedra_aeneas_membership_kernel

/-- Catalog entry: C-old majority is not enough during joint add. -/
theorem joint_election_ok_needs_both :
    joint_election_ok (2#u64) (3#u64) (some (2#u64, 4#u64)) = ok false := by
  unfold joint_election_ok
  unfold majority_of
  rfl

/-- AS-IS dente: C-old majority elects during joint add. -/
theorem joint_election_ok_as_is_dente :
    joint_election_ok_as_is (2#u64) (3#u64) (some (2#u64, 4#u64)) = ok true := by
  unfold joint_election_ok_as_is
  unfold majority_of
  rfl

/-- RFC-0069 P2.2: bounded elect does not print live. -/
theorem elect_claim_banner_bounded :
    elect_claim_banner false false false =
      ok (toStr "bounded-elect not-eventual") := by
  unfold elect_claim_banner
  unfold liveness_admitted
  simp

/-- AS-IS dente: banner is live without naming ES. -/
theorem elect_claim_banner_as_is_dente :
    elect_claim_banner_as_is false false false = ok (toStr "live") := by
  unfold elect_claim_banner_as_is
  simp
