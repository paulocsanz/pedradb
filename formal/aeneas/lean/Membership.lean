-- Theorems over Aeneas extract of raft membership_kernel.rs (Raft §6).
-- Charon --exclude elect_claim_banner (static-str bottoms).
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
