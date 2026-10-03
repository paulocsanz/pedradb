-- Cross-lib: raft membership_kernel.rs and store membership_kernel.rs clones.
import Aeneas
import MembershipKernel
import StoreMembershipKernel
open Aeneas.Std Result

/-- Joint add: C-old majority is not enough on either clone. -/
theorem joint_election_ok_clones_refuse :
    pedra_aeneas_membership_kernel.joint_election_ok
      (2#u64) (3#u64) (some (2#u64, 4#u64))
      = pedra_aeneas_store_membership_kernel.joint_election_ok
          (2#u64) (3#u64) (some (2#u64, 4#u64)) := by
  unfold pedra_aeneas_membership_kernel.joint_election_ok
  unfold pedra_aeneas_membership_kernel.majority_of
  unfold pedra_aeneas_store_membership_kernel.joint_election_ok
  unfold pedra_aeneas_store_membership_kernel.majority_of
  rfl

/-- Single config (no joint): both clones elect on C-old majority. -/
theorem joint_election_ok_clones_single_cfg :
    pedra_aeneas_membership_kernel.joint_election_ok
      (2#u64) (3#u64) none
      = pedra_aeneas_store_membership_kernel.joint_election_ok
          (2#u64) (3#u64) none := by
  unfold pedra_aeneas_membership_kernel.joint_election_ok
  unfold pedra_aeneas_membership_kernel.majority_of
  unfold pedra_aeneas_store_membership_kernel.joint_election_ok
  unfold pedra_aeneas_store_membership_kernel.majority_of
  rfl

/-- AS-IS dente: both clones elect on C-old majority during joint add. -/
theorem joint_election_ok_as_is_clones_agree :
    pedra_aeneas_membership_kernel.joint_election_ok_as_is
      (2#u64) (3#u64) (some (2#u64, 4#u64))
      = pedra_aeneas_store_membership_kernel.joint_election_ok_as_is
          (2#u64) (3#u64) (some (2#u64, 4#u64)) := by
  unfold pedra_aeneas_membership_kernel.joint_election_ok_as_is
  unfold pedra_aeneas_membership_kernel.majority_of
  unfold pedra_aeneas_store_membership_kernel.joint_election_ok_as_is
  unfold pedra_aeneas_store_membership_kernel.majority_of
  rfl
