-- Theorems over Aeneas extract of store membership_kernel.rs.
-- elect_claim_banner &'static str bottoms patched to toStr (clone of raft).
import Aeneas
import StoreMembershipKernel
open Aeneas.Std Result
open pedra_aeneas_store_membership_kernel
open pedra_aeneas_store_membership_kernel.membership_kernel

/-- C-old majority is not enough during joint add. -/
theorem joint_election_ok_needs_both :
    joint_election_ok (2#u64) (3#u64) (some (2#u64, 4#u64)) = ok false := by
  unfold joint_election_ok
  unfold majority_of
  rfl

/-- AS-IS tooth: C-old majority elects during joint add. -/
theorem joint_election_ok_as_is_tooth :
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

/-- AS-IS tooth: banner is live without naming ES. -/
theorem elect_claim_banner_as_is_tooth :
    elect_claim_banner_as_is false false false = ok (toStr "live") := by
  unfold elect_claim_banner_as_is
  simp

/-- Catalog entry: a planted committed-joint schedule is ok exactly
    when the opt-in world emits it and the default world still omits it
    (RFC-0068 — the plant is visible only to the opted-in reader). -/
theorem plant_joint_schedule_ok_iff_opt_in_emits_and_default_omits :
    ∀ (opt_in_emits : Bool) (default_omits : Bool),
      (plant_joint_schedule_ok opt_in_emits default_omits = ok true)
        ↔ (opt_in_emits = true ∧ default_omits = true) := by
  intro opt_in_emits default_omits
  unfold plant_joint_schedule_ok
  constructor
  · intro h
    split at h
    · next c1 =>
      simp at h
      exact ⟨c1, h⟩
    · next c1 => exact absurd h (by simp)
  · rintro ⟨h1, h2⟩
    rw [if_pos h1, h2]
