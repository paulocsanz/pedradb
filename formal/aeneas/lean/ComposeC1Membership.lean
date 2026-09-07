-- Cross-lib: c1_modelo_kernel shim of membership vs raft membership extract.
import Aeneas
import C1ModeloKernel
import MembershipKernel
open Aeneas.Std Result

/-- Joint add: C1's `#[path]` membership agrees with the raft extract. -/
theorem c1_joint_election_matches_membership :
    pedra_aeneas_c1_modelo_kernel.membership_kernel.joint_election_ok
      (2#u64) (3#u64) (some (2#u64, 4#u64))
      = pedra_aeneas_membership_kernel.joint_election_ok
          (2#u64) (3#u64) (some (2#u64, 4#u64)) := by
  unfold pedra_aeneas_c1_modelo_kernel.membership_kernel.joint_election_ok
  unfold pedra_aeneas_c1_modelo_kernel.membership_kernel.majority_of
  unfold pedra_aeneas_membership_kernel.joint_election_ok
  unfold pedra_aeneas_membership_kernel.majority_of
  rfl

/-- Single config: both elect. -/
theorem c1_joint_election_single_cfg_matches_membership :
    pedra_aeneas_c1_modelo_kernel.membership_kernel.joint_election_ok
      (2#u64) (3#u64) none
      = pedra_aeneas_membership_kernel.joint_election_ok
          (2#u64) (3#u64) none := by
  unfold pedra_aeneas_c1_modelo_kernel.membership_kernel.joint_election_ok
  unfold pedra_aeneas_c1_modelo_kernel.membership_kernel.majority_of
  unfold pedra_aeneas_membership_kernel.joint_election_ok
  unfold pedra_aeneas_membership_kernel.majority_of
  rfl

/-- AS-IS dente: both elect on C-old majority during joint add. -/
theorem c1_joint_election_as_is_matches_membership :
    pedra_aeneas_c1_modelo_kernel.membership_kernel.joint_election_ok_as_is
      (2#u64) (3#u64) (some (2#u64, 4#u64))
      = pedra_aeneas_membership_kernel.joint_election_ok_as_is
          (2#u64) (3#u64) (some (2#u64, 4#u64)) := by
  unfold pedra_aeneas_c1_modelo_kernel.membership_kernel.joint_election_ok_as_is
  unfold pedra_aeneas_c1_modelo_kernel.membership_kernel.majority_of
  unfold pedra_aeneas_membership_kernel.joint_election_ok_as_is
  unfold pedra_aeneas_membership_kernel.majority_of
  rfl
