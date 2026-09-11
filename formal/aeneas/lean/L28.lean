-- Theorems over Aeneas extract of l28.rs (L28 world gates, RFC-L28).
-- Payment is the linked rustc bodies; the former cfg(verus_keep_ghost)
-- stand-in was deleted. Fail-closed: this file must not contain a hole.
import Aeneas
import L28Kernel
open Aeneas.Std Result
open pedra_aeneas_l28_kernel

/-- L28 durability teeth: every step ok ends ok. -/
theorem l28_durability_all_ok :
    l28_durability_ok true true true = ok true := by
  unfold l28_durability_ok
  rfl

/-- L28 durability teeth: a failed step after the kill fails closed. -/
theorem l28_durability_kill_step_fails_closed :
    l28_durability_ok true false true = ok false := by
  unfold l28_durability_ok
  rfl

/-- AS-IS L28 dente: the get answer alone decides (kill/restart ignored). -/
theorem l28_durability_as_is_ignores_kill :
    l28_durability_ok_as_is true false true = ok true := by
  rfl

/-- L28 leader-kill delegates to the durability gate (same fail-closed). -/
theorem l28_leader_kill_matches_durability :
    l28_leader_kill_ok true false true = ok false := by
  rfl

/-- AS-IS L28 dente: leader-kill ignores the kill step. -/
theorem l28_leader_kill_as_is_ignores_kill :
    l28_leader_kill_ok_as_is true false true = ok true := by
  rfl

/-- World seed teeth: a silent-wrong seed fails the world even if the
    cluster reports ok. -/
theorem world_seed_l28_ok_silent_wrong_fails :
    world_seed_l28_ok (1#u64) true = ok false := by
  unfold world_seed_l28_ok
  simp

/-- World seed teeth: clean seed, cluster ok ends ok. -/
theorem world_seed_l28_ok_clean_cluster_ok :
    world_seed_l28_ok (0#u64) true = ok true := by
  unfold world_seed_l28_ok
  simp

/-- AS-IS world-seed dente: the cluster verdict is only the seed being
    clean — silent-wrong with ok seed still reports ok. -/
theorem world_seed_l28_ok_as_is_ignores_cluster :
    world_seed_l28_ok_as_is (0#u64) false = ok true := by
  unfold world_seed_l28_ok_as_is
  simp

/-- Plant teeth: a failed remove or a failed leave blocks the plant. -/
theorem l28_tcp_plant_leave_blocks :
    l28_tcp_plant_ok true false = ok false := by
  rfl

/-- Plant teeth: failed remove blocks regardless of leave. -/
theorem l28_tcp_plant_remove_failure_blocks :
    l28_tcp_plant_ok false true = ok false := by
  rfl

/-- AS-IS plant dente: neither remove nor leave is checked. -/
theorem l28_tcp_plant_as_is_ignores_both :
    l28_tcp_plant_ok_as_is false false = ok true := by
  rfl

/-- Retry teeth: a not-applied op is never admitted on retry. -/
theorem l28_tcp_napply_retry_never_admitted :
    l28_tcp_napply_retry_admitted (5#u64) true = ok false := by
  rfl

/-- AS-IS retry dente: first retry of a not-applied op is admitted. -/
theorem l28_tcp_napply_retry_as_is_admits :
    l28_tcp_napply_retry_admitted_as_is (1#u64) true = ok true := by
  unfold l28_tcp_napply_retry_admitted_as_is
  simp

/-- L28 leave gate teeth: the gate propagates the underlying result. -/
theorem l28_tcp_leave_propagates :
    l28_tcp_leave_ok false = ok false ∧ l28_tcp_leave_ok true = ok true := by
  constructor <;> rfl

/-- AS-IS L28 leave dente: the gate is always ok. -/
theorem l28_tcp_leave_as_is_always_ok :
    l28_tcp_leave_ok_as_is false = ok true := by
  rfl

/-- L28 left gate teeth: the gate propagates the underlying result. -/
theorem l28_tcp_left_propagates :
    l28_tcp_left_ok false = ok false ∧ l28_tcp_left_ok true = ok true := by
  constructor <;> rfl

/-- AS-IS L28 left dente: the gate is always ok. -/
theorem l28_tcp_left_as_is_always_ok :
    l28_tcp_left_ok_as_is false = ok true := by
  rfl

/-- L28 hw gate teeth: the gate propagates the underlying result. -/
theorem l28_tcp_hw_propagates :
    l28_tcp_hw_ok false = ok false ∧ l28_tcp_hw_ok true = ok true := by
  constructor <;> rfl

/-- AS-IS L28 hw dente: the gate is always ok. -/
theorem l28_tcp_hw_as_is_always_ok :
    l28_tcp_hw_ok_as_is false = ok true := by
  rfl

/-- L28 part gate teeth: the gate propagates the underlying result. -/
theorem l28_tcp_part_propagates :
    l28_tcp_part_ok false = ok false ∧ l28_tcp_part_ok true = ok true := by
  constructor <;> rfl

/-- AS-IS L28 part dente: the gate is always ok. -/
theorem l28_tcp_part_as_is_always_ok :
    l28_tcp_part_ok_as_is false = ok true := by
  rfl

/-- L28 apply gate teeth: the gate propagates the underlying result. -/
theorem l28_tcp_apply_propagates :
    l28_tcp_apply_ok false = ok false ∧ l28_tcp_apply_ok true = ok true := by
  constructor <;> rfl

/-- AS-IS L28 apply dente: the gate is always ok. -/
theorem l28_tcp_apply_as_is_always_ok :
    l28_tcp_apply_ok_as_is false = ok true := by
  rfl

/-- L28 napply gate teeth: the gate propagates the underlying result. -/
theorem l28_tcp_napply_propagates :
    l28_tcp_napply_ok false = ok false ∧ l28_tcp_napply_ok true = ok true := by
  constructor <;> rfl

/-- AS-IS L28 napply dente: the gate is always ok. -/
theorem l28_tcp_napply_as_is_always_ok :
    l28_tcp_napply_ok_as_is false = ok true := by
  rfl

/-- L28 trunc gate teeth: the gate propagates the underlying result. -/
theorem l28_tcp_trunc_propagates :
    l28_tcp_trunc_ok false = ok false ∧ l28_tcp_trunc_ok true = ok true := by
  constructor <;> rfl

/-- AS-IS L28 trunc dente: the gate is always ok. -/
theorem l28_tcp_trunc_as_is_always_ok :
    l28_tcp_trunc_ok_as_is false = ok true := by
  rfl

/-- L28 odrop gate teeth: the gate propagates the underlying result. -/
theorem l28_tcp_odrop_propagates :
    l28_tcp_odrop_ok false = ok false ∧ l28_tcp_odrop_ok true = ok true := by
  constructor <;> rfl

/-- AS-IS L28 odrop dente: the gate is always ok. -/
theorem l28_tcp_odrop_as_is_always_ok :
    l28_tcp_odrop_ok_as_is false = ok true := by
  rfl

/-- L28 abort gate teeth: the gate propagates the underlying result. -/
theorem l28_tcp_abort_propagates :
    l28_tcp_abort_ok false = ok false ∧ l28_tcp_abort_ok true = ok true := by
  constructor <;> rfl

/-- AS-IS L28 abort dente: the gate is always ok. -/
theorem l28_tcp_abort_as_is_always_ok :
    l28_tcp_abort_ok_as_is false = ok true := by
  rfl

/-- L28 nowms gate teeth: the gate propagates the underlying result. -/
theorem l28_tcp_nowms_propagates :
    l28_tcp_nowms_ok false = ok false ∧ l28_tcp_nowms_ok true = ok true := by
  constructor <;> rfl

/-- AS-IS L28 nowms dente: the gate is always ok. -/
theorem l28_tcp_nowms_as_is_always_ok :
    l28_tcp_nowms_ok_as_is false = ok true := by
  rfl

/-- L28 dterm gate teeth: the gate propagates the underlying result. -/
theorem l28_tcp_dterm_propagates :
    l28_tcp_dterm_ok false = ok false ∧ l28_tcp_dterm_ok true = ok true := by
  constructor <;> rfl

/-- AS-IS L28 dterm dente: the gate is always ok. -/
theorem l28_tcp_dterm_as_is_always_ok :
    l28_tcp_dterm_ok_as_is false = ok true := by
  rfl

/-- L28 hist gate teeth: the gate propagates the underlying result. -/
theorem l28_tcp_hist_propagates :
    l28_tcp_hist_ok false = ok false ∧ l28_tcp_hist_ok true = ok true := by
  constructor <;> rfl

/-- AS-IS L28 hist dente: the gate is always ok. -/
theorem l28_tcp_hist_as_is_always_ok :
    l28_tcp_hist_ok_as_is false = ok true := by
  rfl

/-- L28 fence gate teeth: the gate propagates the underlying result. -/
theorem l28_tcp_fence_propagates :
    l28_tcp_fence_ok false = ok false ∧ l28_tcp_fence_ok true = ok true := by
  constructor <;> rfl

/-- AS-IS L28 fence dente: the gate is always ok. -/
theorem l28_tcp_fence_as_is_always_ok :
    l28_tcp_fence_ok_as_is false = ok true := by
  rfl

/-- L28 clear gate teeth: the gate propagates the underlying result. -/
theorem l28_tcp_clear_propagates :
    l28_tcp_clear_ok false = ok false ∧ l28_tcp_clear_ok true = ok true := by
  constructor <;> rfl

/-- AS-IS L28 clear dente: the gate is always ok. -/
theorem l28_tcp_clear_as_is_always_ok :
    l28_tcp_clear_ok_as_is false = ok true := by
  rfl

/-- L28 pre gate teeth: the gate propagates the underlying result. -/
theorem l28_tcp_pre_propagates :
    l28_tcp_pre_ok false = ok false ∧ l28_tcp_pre_ok true = ok true := by
  constructor <;> rfl

/-- AS-IS L28 pre dente: the gate is always ok. -/
theorem l28_tcp_pre_as_is_always_ok :
    l28_tcp_pre_ok_as_is false = ok true := by
  rfl

/-- L28 peer gate teeth: the gate propagates the underlying result. -/
theorem l28_tcp_peer_propagates :
    l28_tcp_peer_ok false = ok false ∧ l28_tcp_peer_ok true = ok true := by
  constructor <;> rfl

/-- AS-IS L28 peer dente: the gate is always ok. -/
theorem l28_tcp_peer_as_is_always_ok :
    l28_tcp_peer_ok_as_is false = ok true := by
  rfl

/-- L28 lid gate teeth: the gate propagates the underlying result. -/
theorem l28_tcp_lid_propagates :
    l28_tcp_lid_ok false = ok false ∧ l28_tcp_lid_ok true = ok true := by
  constructor <;> rfl

/-- AS-IS L28 lid dente: the gate is always ok. -/
theorem l28_tcp_lid_as_is_always_ok :
    l28_tcp_lid_ok_as_is false = ok true := by
  rfl

/-- L28 rdr gate teeth: the gate propagates the underlying result. -/
theorem l28_tcp_rdr_propagates :
    l28_tcp_rdr_ok false = ok false ∧ l28_tcp_rdr_ok true = ok true := by
  constructor <;> rfl

/-- AS-IS L28 rdr dente: the gate is always ok. -/
theorem l28_tcp_rdr_as_is_always_ok :
    l28_tcp_rdr_ok_as_is false = ok true := by
  rfl

/-- L28 dsc gate teeth: the gate propagates the underlying result. -/
theorem l28_tcp_dsc_propagates :
    l28_tcp_dsc_ok false = ok false ∧ l28_tcp_dsc_ok true = ok true := by
  constructor <;> rfl

/-- AS-IS L28 dsc dente: the gate is always ok. -/
theorem l28_tcp_dsc_as_is_always_ok :
    l28_tcp_dsc_ok_as_is false = ok true := by
  rfl

/-- L28 pld gate teeth: the gate propagates the underlying result. -/
theorem l28_tcp_pld_propagates :
    l28_tcp_pld_ok false = ok false ∧ l28_tcp_pld_ok true = ok true := by
  constructor <;> rfl

/-- AS-IS L28 pld dente: the gate is always ok. -/
theorem l28_tcp_pld_as_is_always_ok :
    l28_tcp_pld_ok_as_is false = ok true := by
  rfl

/-- L28 std gate teeth: the gate propagates the underlying result. -/
theorem l28_tcp_std_propagates :
    l28_tcp_std_ok false = ok false ∧ l28_tcp_std_ok true = ok true := by
  constructor <;> rfl

/-- AS-IS L28 std dente: the gate is always ok. -/
theorem l28_tcp_std_as_is_always_ok :
    l28_tcp_std_ok_as_is false = ok true := by
  rfl

/-- L28 hnt gate teeth: the gate propagates the underlying result. -/
theorem l28_tcp_hnt_propagates :
    l28_tcp_hnt_ok false = ok false ∧ l28_tcp_hnt_ok true = ok true := by
  constructor <;> rfl

/-- AS-IS L28 hnt dente: the gate is always ok. -/
theorem l28_tcp_hnt_as_is_always_ok :
    l28_tcp_hnt_ok_as_is false = ok true := by
  rfl

/-- L28 slot gate teeth: the gate propagates the underlying result. -/
theorem l28_tcp_slot_propagates :
    l28_tcp_slot_ok false = ok false ∧ l28_tcp_slot_ok true = ok true := by
  constructor <;> rfl

/-- AS-IS L28 slot dente: the gate is always ok. -/
theorem l28_tcp_slot_as_is_always_ok :
    l28_tcp_slot_ok_as_is false = ok true := by
  rfl

/-- L28 sth gate teeth: the gate propagates the underlying result. -/
theorem l28_tcp_sth_propagates :
    l28_tcp_sth_ok false = ok false ∧ l28_tcp_sth_ok true = ok true := by
  constructor <;> rfl

/-- AS-IS L28 sth dente: the gate is always ok. -/
theorem l28_tcp_sth_as_is_always_ok :
    l28_tcp_sth_ok_as_is false = ok true := by
  rfl

/-- L28 pj gate teeth: the gate propagates the underlying result. -/
theorem l28_tcp_pj_propagates :
    l28_tcp_pj_ok false = ok false ∧ l28_tcp_pj_ok true = ok true := by
  constructor <;> rfl

/-- AS-IS L28 pj dente: the gate is always ok. -/
theorem l28_tcp_pj_as_is_always_ok :
    l28_tcp_pj_ok_as_is false = ok true := by
  rfl

/-- RFC-0208 P2.1 (l28 band promotion 1/2, atom
    `catalog:l28_tcp_left`): the real-TCP node reports "member
    left on disk" EXACTLY when the disk says so — the removal is
    never reported when it did not happen, and never hidden when it
    did — fate forall over the extracted pure-lift body; the AS-IS
    `ok true` mutant reports removal unconditionally (the lie the
    real TCP plant `l28_real_tcp_remove_member_left_on_disk`
    refutes). -/
theorem l28_tcp_left_ok_fate_iff :
    ∀ (left : Bool) (v : Bool),
      (l28_tcp_left_ok left = ok v) ↔
        ((v = true ∧ left = true)
          ∨ (v = false ∧ left = false)) := by
  intro left v
  unfold l28_tcp_left_ok
  cases left <;> cases v <;> simp

/-- RFC-0208 P2.1 (l28 band promotion 2/2, atom
    `catalog:l28_tcp_hw`): after a removal the real-TCP node's
    high-water moves EXACTLY when the committed inventory was kept
    — the durable progress survived the removal — fate forall over
    the extracted pure-lift body; the AS-IS `ok true` mutant claims
    the high-water always moved (the lie the real TCP plant
    `l28_real_tcp_high_water_after_remove` refutes). -/
theorem l28_tcp_hw_ok_fate_iff :
    ∀ (kept : Bool) (v : Bool),
      (l28_tcp_hw_ok kept = ok v) ↔
        ((v = true ∧ kept = true)
          ∨ (v = false ∧ kept = false)) := by
  intro kept v
  unfold l28_tcp_hw_ok
  cases kept <;> cases v <;> simp

/-- RFC-0210 P0.1 (l28 cadence 1/4, atom `catalog:l28_tcp_dterm`):
    on the removed replica's REAL dir, a newer-term RequestVote whose
    hard-state persist fails rolls the term back EXACTLY when the
    rollback held — reply, memory and disk keep the previous term
    (F125/F127) — fate forall over the extracted pure-lift body; the
    AS-IS `ok true` mutant keeps the undurable raise (memory term
    above disk hard state — the lie the real TCP plant
    `l28_real_tcp_removed_durable_term` refutes). -/
theorem l28_tcp_dterm_ok_fate_iff :
    ∀ (b : Bool) (v : Bool),
      (l28_tcp_dterm_ok b = ok v) ↔
        ((v = true ∧ b = true)
          ∨ (v = false ∧ b = false)) := by
  intro b v
  unfold l28_tcp_dterm_ok
  cases b <;> cases v <;> simp

/-- RFC-0210 P0.1 (l28 cadence 1/4, atom `catalog:l28_tcp_part`):
    after a removal, a removed voter is reported participating
    EXACTLY when the participating scan says so — a stale CLI/nodes
    map must not count it — fate forall over the extracted pure-lift
    body; the AS-IS `ok true` mutant skips the scan (the 0127
    leftover: reopen flag only — the lie the real TCP plant
    `l28_real_tcp_participating_after_remove` refutes). -/
theorem l28_tcp_part_ok_fate_iff :
    ∀ (b : Bool) (v : Bool),
      (l28_tcp_part_ok b = ok v) ↔
        ((v = true ∧ b = true)
          ∨ (v = false ∧ b = false)) := by
  intro b v
  unfold l28_tcp_part_ok
  cases b <;> cases v <;> simp
