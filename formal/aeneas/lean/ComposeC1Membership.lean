-- Cross-lib: c1_modelo (production caller) vs raft membership extract.
import Aeneas
import C1ModeloKernel
import MembershipKernel
open Aeneas.Std Result

private def jointAdd : pedra_aeneas_c1_modelo_kernel.c1_modelo_kernel.C1State :=
  { old_n := 3#u64, old_yes := 2#u64, joint := true, new_n := 4#u64,
    new_yes := 1#u64, current_term := 7#u64, index_term := 7#u64,
    commit_index := 0#u64, proposed := 5#u64, served := true }

/-- Joint add: `c1_modelo` refuses, and the membership extract refuses the same votes. -/
theorem c1_modelo_joint_add_via_membership_extract :
    pedra_aeneas_c1_modelo_kernel.c1_modelo_kernel.c1_modelo jointAdd = ok false
    ∧ pedra_aeneas_membership_kernel.joint_election_ok
        (2#u64) (3#u64) (some (1#u64, 4#u64)) = ok false := by
  constructor
  · unfold pedra_aeneas_c1_modelo_kernel.c1_modelo_kernel.c1_modelo
    unfold pedra_aeneas_c1_modelo_kernel.c1_modelo_kernel.c1_advance_commit
    unfold pedra_aeneas_c1_modelo_kernel.c1_modelo_kernel.c1_quorum
    unfold pedra_aeneas_c1_modelo_kernel.c1_modelo_kernel.new_cfg
    unfold pedra_aeneas_c1_modelo_kernel.membership_kernel.joint_election_ok
    unfold pedra_aeneas_c1_modelo_kernel.membership_kernel.majority_of
    unfold pedra_aeneas_c1_modelo_kernel.commit_kernel.may_commit_at
    unfold pedra_aeneas_c1_modelo_kernel.commit_kernel.propose_ack_ok
    rfl
  · unfold pedra_aeneas_membership_kernel.joint_election_ok
    unfold pedra_aeneas_membership_kernel.majority_of
    rfl

/-- Single config: `c1_modelo` with no joint elects, membership extract elects. -/
theorem c1_modelo_single_cfg_via_membership_extract :
    pedra_aeneas_c1_modelo_kernel.c1_modelo_kernel.c1_modelo
        { jointAdd with joint := false } = ok true
    ∧ pedra_aeneas_membership_kernel.joint_election_ok
        (2#u64) (3#u64) none = ok true := by
  constructor
  · unfold pedra_aeneas_c1_modelo_kernel.c1_modelo_kernel.c1_modelo
    unfold pedra_aeneas_c1_modelo_kernel.c1_modelo_kernel.c1_advance_commit
    unfold pedra_aeneas_c1_modelo_kernel.c1_modelo_kernel.c1_quorum
    unfold pedra_aeneas_c1_modelo_kernel.c1_modelo_kernel.new_cfg
    unfold pedra_aeneas_c1_modelo_kernel.membership_kernel.joint_election_ok
    unfold pedra_aeneas_c1_modelo_kernel.membership_kernel.majority_of
    unfold pedra_aeneas_c1_modelo_kernel.commit_kernel.may_commit_at
    unfold pedra_aeneas_c1_modelo_kernel.commit_kernel.propose_ack_ok
    rfl
  · unfold pedra_aeneas_membership_kernel.joint_election_ok
    unfold pedra_aeneas_membership_kernel.majority_of
    rfl

/-- AS-IS tooth: `c1_modelo_as_is` serves, and membership as-is elects on C-old. -/
theorem c1_modelo_as_is_via_membership_as_is :
    pedra_aeneas_c1_modelo_kernel.c1_modelo_kernel.c1_modelo_as_is jointAdd
      = ok true
    ∧ pedra_aeneas_membership_kernel.joint_election_ok_as_is
        (2#u64) (3#u64) (some (1#u64, 4#u64)) = ok true := by
  constructor
  · unfold pedra_aeneas_c1_modelo_kernel.c1_modelo_kernel.c1_modelo_as_is
    unfold pedra_aeneas_c1_modelo_kernel.c1_modelo_kernel.c1_advance_commit_as_is
    unfold pedra_aeneas_c1_modelo_kernel.c1_modelo_kernel.new_cfg
    unfold pedra_aeneas_c1_modelo_kernel.membership_kernel.joint_election_ok_as_is
    unfold pedra_aeneas_c1_modelo_kernel.membership_kernel.majority_of
    unfold pedra_aeneas_c1_modelo_kernel.commit_kernel.may_commit_at_as_is
    unfold pedra_aeneas_c1_modelo_kernel.commit_kernel.propose_ack_ok_as_is
    rfl
  · unfold pedra_aeneas_membership_kernel.joint_election_ok_as_is
    unfold pedra_aeneas_membership_kernel.majority_of
    rfl
