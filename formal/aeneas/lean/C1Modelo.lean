-- Theorems over Aeneas extract of c1_modelo_kernel.rs (RFC-0166 P2.3).
import Aeneas
import C1ModeloKernel
open Aeneas.Std Result
open pedra_aeneas_c1_modelo_kernel

private def jointAdd : c1_modelo_kernel.C1State :=
  { old_n := 3#u64, old_yes := 2#u64, joint := true, new_n := 4#u64,
    new_yes := 1#u64, current_term := 7#u64, index_term := 7#u64,
    commit_index := 0#u64, proposed := 5#u64, served := true }

/-- Catalog entry: joint-add shape does not serve. -/
theorem c1_modelo_joint_add_refuses :
    c1_modelo_kernel.c1_modelo jointAdd = ok false := by
  unfold c1_modelo_kernel.c1_modelo
  unfold c1_modelo_kernel.c1_advance_commit
  unfold c1_modelo_kernel.c1_quorum
  unfold c1_modelo_kernel.new_cfg
  unfold membership_kernel.joint_election_ok
  unfold membership_kernel.majority_of
  unfold commit_kernel.may_commit_at
  unfold commit_kernel.propose_ack_ok
  rfl

/-- AS-IS dente: C-old majority acks during joint add. -/
theorem c1_modelo_as_is_dente :
    c1_modelo_kernel.c1_modelo_as_is jointAdd = ok true := by
  unfold c1_modelo_kernel.c1_modelo_as_is
  unfold c1_modelo_kernel.c1_advance_commit_as_is
  unfold c1_modelo_kernel.new_cfg
  unfold membership_kernel.joint_election_ok_as_is
  unfold membership_kernel.majority_of
  unfold commit_kernel.may_commit_at_as_is
  unfold commit_kernel.propose_ack_ok_as_is
  rfl
