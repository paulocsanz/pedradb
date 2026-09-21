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

/-- AS-IS L28 tooth: the get answer alone decides (kill/restart ignored). -/
theorem l28_durability_as_is_ignores_kill :
    l28_durability_ok_as_is true false true = ok true := by
  rfl

/-- L28 leader-kill delegates to the durability gate (same fail-closed). -/
theorem l28_leader_kill_matches_durability :
    l28_leader_kill_ok true false true = ok false := by
  rfl

/-- AS-IS L28 tooth: leader-kill ignores the kill step. -/
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

/-- AS-IS world-seed tooth: the cluster verdict is only the seed being
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

/-- AS-IS plant tooth: neither remove nor leave is checked. -/
theorem l28_tcp_plant_as_is_ignores_both :
    l28_tcp_plant_ok_as_is false false = ok true := by
  rfl

/-- Retry teeth: a not-applied op is never admitted on retry. -/
theorem l28_tcp_napply_retry_never_admitted :
    l28_tcp_napply_retry_admitted (5#u64) true = ok false := by
  rfl

/-- AS-IS retry tooth: first retry of a not-applied op is admitted. -/
theorem l28_tcp_napply_retry_as_is_admits :
    l28_tcp_napply_retry_admitted_as_is (1#u64) true = ok true := by
  unfold l28_tcp_napply_retry_admitted_as_is
  simp

/-- L28 leave gate teeth: the gate propagates the underlying result. -/
theorem l28_tcp_leave_propagates :
    l28_tcp_leave_ok false = ok false ∧ l28_tcp_leave_ok true = ok true := by
  constructor <;> rfl

/-- AS-IS L28 leave tooth: the gate is always ok. -/
theorem l28_tcp_leave_as_is_always_ok :
    l28_tcp_leave_ok_as_is false = ok true := by
  rfl

/-- L28 left gate teeth: the gate propagates the underlying result. -/
theorem l28_tcp_left_propagates :
    l28_tcp_left_ok false = ok false ∧ l28_tcp_left_ok true = ok true := by
  constructor <;> rfl

/-- AS-IS L28 left tooth: the gate is always ok. -/
theorem l28_tcp_left_as_is_always_ok :
    l28_tcp_left_ok_as_is false = ok true := by
  rfl

/-- L28 hw gate teeth: the gate propagates the underlying result. -/
theorem l28_tcp_hw_propagates :
    l28_tcp_hw_ok false = ok false ∧ l28_tcp_hw_ok true = ok true := by
  constructor <;> rfl

/-- AS-IS L28 hw tooth: the gate is always ok. -/
theorem l28_tcp_hw_as_is_always_ok :
    l28_tcp_hw_ok_as_is false = ok true := by
  rfl

/-- L28 part gate teeth: the gate propagates the underlying result. -/
theorem l28_tcp_part_propagates :
    l28_tcp_part_ok false = ok false ∧ l28_tcp_part_ok true = ok true := by
  constructor <;> rfl

/-- AS-IS L28 part tooth: the gate is always ok. -/
theorem l28_tcp_part_as_is_always_ok :
    l28_tcp_part_ok_as_is false = ok true := by
  rfl

/-- L28 apply gate teeth: the gate propagates the underlying result. -/
theorem l28_tcp_apply_propagates :
    l28_tcp_apply_ok false = ok false ∧ l28_tcp_apply_ok true = ok true := by
  constructor <;> rfl

/-- AS-IS L28 apply tooth: the gate is always ok. -/
theorem l28_tcp_apply_as_is_always_ok :
    l28_tcp_apply_ok_as_is false = ok true := by
  rfl

/-- L28 napply gate teeth: the gate propagates the underlying result. -/
theorem l28_tcp_napply_propagates :
    l28_tcp_napply_ok false = ok false ∧ l28_tcp_napply_ok true = ok true := by
  constructor <;> rfl

/-- AS-IS L28 napply tooth: the gate is always ok. -/
theorem l28_tcp_napply_as_is_always_ok :
    l28_tcp_napply_ok_as_is false = ok true := by
  rfl

/-- L28 trunc gate teeth: the gate propagates the underlying result. -/
theorem l28_tcp_trunc_propagates :
    l28_tcp_trunc_ok false = ok false ∧ l28_tcp_trunc_ok true = ok true := by
  constructor <;> rfl

/-- AS-IS L28 trunc tooth: the gate is always ok. -/
theorem l28_tcp_trunc_as_is_always_ok :
    l28_tcp_trunc_ok_as_is false = ok true := by
  rfl

/-- L28 odrop gate teeth: the gate propagates the underlying result. -/
theorem l28_tcp_odrop_propagates :
    l28_tcp_odrop_ok false = ok false ∧ l28_tcp_odrop_ok true = ok true := by
  constructor <;> rfl

/-- AS-IS L28 odrop tooth: the gate is always ok. -/
theorem l28_tcp_odrop_as_is_always_ok :
    l28_tcp_odrop_ok_as_is false = ok true := by
  rfl

/-- L28 abort gate teeth: the gate propagates the underlying result. -/
theorem l28_tcp_abort_propagates :
    l28_tcp_abort_ok false = ok false ∧ l28_tcp_abort_ok true = ok true := by
  constructor <;> rfl

/-- AS-IS L28 abort tooth: the gate is always ok. -/
theorem l28_tcp_abort_as_is_always_ok :
    l28_tcp_abort_ok_as_is false = ok true := by
  rfl

/-- L28 nowms gate teeth: the gate propagates the underlying result. -/
theorem l28_tcp_nowms_propagates :
    l28_tcp_nowms_ok false = ok false ∧ l28_tcp_nowms_ok true = ok true := by
  constructor <;> rfl

/-- AS-IS L28 nowms tooth: the gate is always ok. -/
theorem l28_tcp_nowms_as_is_always_ok :
    l28_tcp_nowms_ok_as_is false = ok true := by
  rfl

/-- L28 dterm gate teeth: the gate propagates the underlying result. -/
theorem l28_tcp_dterm_propagates :
    l28_tcp_dterm_ok false = ok false ∧ l28_tcp_dterm_ok true = ok true := by
  constructor <;> rfl

/-- AS-IS L28 dterm tooth: the gate is always ok. -/
theorem l28_tcp_dterm_as_is_always_ok :
    l28_tcp_dterm_ok_as_is false = ok true := by
  rfl

/-- L28 hist gate teeth: the gate propagates the underlying result. -/
theorem l28_tcp_hist_propagates :
    l28_tcp_hist_ok false = ok false ∧ l28_tcp_hist_ok true = ok true := by
  constructor <;> rfl

/-- AS-IS L28 hist tooth: the gate is always ok. -/
theorem l28_tcp_hist_as_is_always_ok :
    l28_tcp_hist_ok_as_is false = ok true := by
  rfl

/-- L28 fence gate teeth: the gate propagates the underlying result. -/
theorem l28_tcp_fence_propagates :
    l28_tcp_fence_ok false = ok false ∧ l28_tcp_fence_ok true = ok true := by
  constructor <;> rfl

/-- AS-IS L28 fence tooth: the gate is always ok. -/
theorem l28_tcp_fence_as_is_always_ok :
    l28_tcp_fence_ok_as_is false = ok true := by
  rfl

/-- L28 clear gate teeth: the gate propagates the underlying result. -/
theorem l28_tcp_clear_propagates :
    l28_tcp_clear_ok false = ok false ∧ l28_tcp_clear_ok true = ok true := by
  constructor <;> rfl

/-- AS-IS L28 clear tooth: the gate is always ok. -/
theorem l28_tcp_clear_as_is_always_ok :
    l28_tcp_clear_ok_as_is false = ok true := by
  rfl

/-- L28 pre gate teeth: the gate propagates the underlying result. -/
theorem l28_tcp_pre_propagates :
    l28_tcp_pre_ok false = ok false ∧ l28_tcp_pre_ok true = ok true := by
  constructor <;> rfl

/-- AS-IS L28 pre tooth: the gate is always ok. -/
theorem l28_tcp_pre_as_is_always_ok :
    l28_tcp_pre_ok_as_is false = ok true := by
  rfl

/-- L28 peer gate teeth: the gate propagates the underlying result. -/
theorem l28_tcp_peer_propagates :
    l28_tcp_peer_ok false = ok false ∧ l28_tcp_peer_ok true = ok true := by
  constructor <;> rfl

/-- AS-IS L28 peer tooth: the gate is always ok. -/
theorem l28_tcp_peer_as_is_always_ok :
    l28_tcp_peer_ok_as_is false = ok true := by
  rfl

/-- L28 lid gate teeth: the gate propagates the underlying result. -/
theorem l28_tcp_lid_propagates :
    l28_tcp_lid_ok false = ok false ∧ l28_tcp_lid_ok true = ok true := by
  constructor <;> rfl

/-- AS-IS L28 lid tooth: the gate is always ok. -/
theorem l28_tcp_lid_as_is_always_ok :
    l28_tcp_lid_ok_as_is false = ok true := by
  rfl

/-- L28 rdr gate teeth: the gate propagates the underlying result. -/
theorem l28_tcp_rdr_propagates :
    l28_tcp_rdr_ok false = ok false ∧ l28_tcp_rdr_ok true = ok true := by
  constructor <;> rfl

/-- AS-IS L28 rdr tooth: the gate is always ok. -/
theorem l28_tcp_rdr_as_is_always_ok :
    l28_tcp_rdr_ok_as_is false = ok true := by
  rfl

/-- L28 dsc gate teeth: the gate propagates the underlying result. -/
theorem l28_tcp_dsc_propagates :
    l28_tcp_dsc_ok false = ok false ∧ l28_tcp_dsc_ok true = ok true := by
  constructor <;> rfl

/-- AS-IS L28 dsc tooth: the gate is always ok. -/
theorem l28_tcp_dsc_as_is_always_ok :
    l28_tcp_dsc_ok_as_is false = ok true := by
  rfl

/-- L28 pld gate teeth: the gate propagates the underlying result. -/
theorem l28_tcp_pld_propagates :
    l28_tcp_pld_ok false = ok false ∧ l28_tcp_pld_ok true = ok true := by
  constructor <;> rfl

/-- AS-IS L28 pld tooth: the gate is always ok. -/
theorem l28_tcp_pld_as_is_always_ok :
    l28_tcp_pld_ok_as_is false = ok true := by
  rfl

/-- L28 std gate teeth: the gate propagates the underlying result. -/
theorem l28_tcp_std_propagates :
    l28_tcp_std_ok false = ok false ∧ l28_tcp_std_ok true = ok true := by
  constructor <;> rfl

/-- AS-IS L28 std tooth: the gate is always ok. -/
theorem l28_tcp_std_as_is_always_ok :
    l28_tcp_std_ok_as_is false = ok true := by
  rfl

/-- L28 hnt gate teeth: the gate propagates the underlying result. -/
theorem l28_tcp_hnt_propagates :
    l28_tcp_hnt_ok false = ok false ∧ l28_tcp_hnt_ok true = ok true := by
  constructor <;> rfl

/-- AS-IS L28 hnt tooth: the gate is always ok. -/
theorem l28_tcp_hnt_as_is_always_ok :
    l28_tcp_hnt_ok_as_is false = ok true := by
  rfl

/-- L28 slot gate teeth: the gate propagates the underlying result. -/
theorem l28_tcp_slot_propagates :
    l28_tcp_slot_ok false = ok false ∧ l28_tcp_slot_ok true = ok true := by
  constructor <;> rfl

/-- AS-IS L28 slot tooth: the gate is always ok. -/
theorem l28_tcp_slot_as_is_always_ok :
    l28_tcp_slot_ok_as_is false = ok true := by
  rfl

/-- L28 sth gate teeth: the gate propagates the underlying result. -/
theorem l28_tcp_sth_propagates :
    l28_tcp_sth_ok false = ok false ∧ l28_tcp_sth_ok true = ok true := by
  constructor <;> rfl

/-- AS-IS L28 sth tooth: the gate is always ok. -/
theorem l28_tcp_sth_as_is_always_ok :
    l28_tcp_sth_ok_as_is false = ok true := by
  rfl

/-- L28 pj gate teeth: the gate propagates the underlying result. -/
theorem l28_tcp_pj_propagates :
    l28_tcp_pj_ok false = ok false ∧ l28_tcp_pj_ok true = ok true := by
  constructor <;> rfl

/-- AS-IS L28 pj tooth: the gate is always ok. -/
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

/-- RFC-0210 P0.1 (l28 cadence 1/4, atom `catalog:l28_tcp_apply`):
    after a REAL TCP plant + process death, the recover-apply closes
    `commit > applied` EXACTLY when the recovery applied it
    (production TCP ctor) — fate forall over the extracted pure-lift
    body; the AS-IS `ok true` mutant skips recover apply (the 0129
    leftover: committed joint stays C-old — the lie the real TCP
    plant `l28_real_tcp_recover_apply` refutes). -/
theorem l28_tcp_apply_ok_fate_iff :
    ∀ (b : Bool) (v : Bool),
      (l28_tcp_apply_ok b = ok v) ↔
        ((v = true ∧ b = true)
          ∨ (v = false ∧ b = false)) := by
  intro b v
  unfold l28_tcp_apply_ok
  cases b <;> cases v <;> simp

/-- RFC-0210 P0.1 (l28 cadence 1/4, atom `catalog:l28_tcp_napply`):
    after a REAL TCP plant + process death, recover apply closes
    `commit > applied` on a replica ALREADY DROPPED from `ids`
    EXACTLY when the recovery applied it — fate forall over the
    extracted pure-lift body; the AS-IS `ok true` mutant skips the
    removed-replica recover apply (the 0130 leftover: ids only —
    the lie the real TCP plant `l28_real_tcp_removed_recover_apply`
    refutes). -/
theorem l28_tcp_napply_ok_fate_iff :
    ∀ (b : Bool) (v : Bool),
      (l28_tcp_napply_ok b = ok v) ↔
        ((v = true ∧ b = true)
          ∨ (v = false ∧ b = false)) := by
  intro b v
  unfold l28_tcp_napply_ok
  cases b <;> cases v <;> simp

/-- RFC-0210 P0.2 (l28 cadence 2/4, atom `catalog:l28_tcp_trunc`):
    after a REAL TCP plant + process death, recover truncate
    persists so disk has NO `index > commit` on a replica dropped
    from `ids` EXACTLY when the truncate persisted — fate forall
    over the extracted pure-lift body; the AS-IS `ok true` mutant
    skips the removed-replica truncate persist (the 0131 leftover:
    ids only — the lie the real TCP plant
    `l28_real_tcp_removed_truncate` refutes). -/
theorem l28_tcp_trunc_ok_fate_iff :
    ∀ (b : Bool) (v : Bool),
      (l28_tcp_trunc_ok b = ok v) ↔
        ((v = true ∧ b = true)
          ∨ (v = false ∧ b = false)) := by
  intro b v
  unfold l28_tcp_trunc_ok
  cases b <;> cases v <;> simp

/-- RFC-0210 P0.2 (l28 cadence 2/4, atom `catalog:l28_tcp_odrop`):
    after a REAL TCP plant + process death, recover truncate
    deletes `log_entry_key` rows past the new hi on a replica
    dropped from `ids` EXACTLY when the orphan rows were dropped —
    fate forall over the extracted pure-lift body; the AS-IS
    `ok true` mutant skips the orphan-segment drop (the 0132
    leftover: watermark only — the lie the real TCP plant
    `l28_real_tcp_removed_orphan_drop` refutes). -/
theorem l28_tcp_odrop_ok_fate_iff :
    ∀ (b : Bool) (v : Bool),
      (l28_tcp_odrop_ok b = ok v) ↔
        ((v = true ∧ b = true)
          ∨ (v = false ∧ b = false)) := by
  intro b v
  unfold l28_tcp_odrop_ok
  cases b <;> cases v <;> simp

/-- RFC-0210 P0.2 (l28 cadence 2/4, atom `catalog:l28_tcp_abort`):
    after a REAL TCP plant + process death, recover abort deletes
    the leftover 2PC intents on a replica dropped from `ids`
    EXACTLY when the abort deleted them — fate forall over the
    extracted pure-lift body; the AS-IS `ok true` mutant skips the
    leftover abort (the 0133 leftover: ids only — the lie the real
    TCP plant `l28_real_tcp_removed_abort` refutes). -/
theorem l28_tcp_abort_ok_fate_iff :
    ∀ (b : Bool) (v : Bool),
      (l28_tcp_abort_ok b = ok v) ↔
        ((v = true ∧ b = true)
          ∨ (v = false ∧ b = false)) := by
  intro b v
  unfold l28_tcp_abort_ok
  cases b <;> cases v <;> simp

/-- RFC-0210 P0.2 (l28 cadence 2/4, atom `catalog:l28_tcp_nowms`):
    after a REAL TCP plant + process death, `now_ms` is persisted on
    a replica dropped from `ids` EXACTLY when the persist happened —
    fate forall over the extracted pure-lift body; the AS-IS
    `ok true` mutant skips the now_ms persist (the 0134 leftover:
    ids only — the lie the real TCP plant
    `l28_real_tcp_removed_now_ms` refutes). -/
theorem l28_tcp_nowms_ok_fate_iff :
    ∀ (b : Bool) (v : Bool),
      (l28_tcp_nowms_ok b = ok v) ↔
        ((v = true ∧ b = true)
          ∨ (v = false ∧ b = false)) := by
  intro b v
  unfold l28_tcp_nowms_ok
  cases b <;> cases v <;> simp

/-- RFC-0210 P1.1 (l28 cadence 3/4, atom `catalog:l28_tcp_hist`):
    after a REAL TCP plant + process death, SI hist is persisted on
    a replica dropped from `ids` EXACTLY when the persist happened —
    fate forall over the extracted pure-lift body; the AS-IS
    `ok true` mutant skips the SI hist persist (the 0135 leftover:
    ids only — the lie the real TCP plant
    `l28_real_tcp_removed_hist` refutes). -/
theorem l28_tcp_hist_ok_fate_iff :
    ∀ (b : Bool) (v : Bool),
      (l28_tcp_hist_ok b = ok v) ↔
        ((v = true ∧ b = true)
          ∨ (v = false ∧ b = false)) := by
  intro b v
  unfold l28_tcp_hist_ok
  cases b <;> cases v <;> simp

/-- RFC-0210 P1.1 (l28 cadence 3/4, atom `catalog:l28_tcp_fence`):
    after a REAL TCP plant + process death, the abort fence is
    persisted on a replica dropped from `ids` EXACTLY when the
    persist happened — fate forall over the extracted pure-lift
    body; the AS-IS `ok true` mutant skips the abort-fence persist
    (the 0136 leftover: ids only — the lie the real TCP plant
    `l28_real_tcp_removed_fence` refutes). -/
theorem l28_tcp_fence_ok_fate_iff :
    ∀ (b : Bool) (v : Bool),
      (l28_tcp_fence_ok b = ok v) ↔
        ((v = true ∧ b = true)
          ∨ (v = false ∧ b = false)) := by
  intro b v
  unfold l28_tcp_fence_ok
  cases b <;> cases v <;> simp

/-- RFC-0210 P1.1 (l28 cadence 3/4, atom `catalog:l28_tcp_clear`):
    after a REAL TCP plant + process death, force-local TX clear
    drops the stuck intents on a replica dropped from `ids`
    EXACTLY when the clear dropped them — fate forall over the
    extracted pure-lift body; the AS-IS `ok true` mutant skips the
    force-local clear (the 0137 leftover: ids only — the lie the
    real TCP plant `l28_real_tcp_removed_clear` refutes). -/
theorem l28_tcp_clear_ok_fate_iff :
    ∀ (b : Bool) (v : Bool),
      (l28_tcp_clear_ok b = ok v) ↔
        ((v = true ∧ b = true)
          ∨ (v = false ∧ b = false)) := by
  intro b v
  unfold l28_tcp_clear_ok
  cases b <;> cases v <;> simp

/-- RFC-0210 P1.1 (l28 cadence 3/4, atom `catalog:l28_tcp_pre`):
    after a REAL TCP plant + process death, TX preimages are
    dropped on a replica dropped from `ids` EXACTLY when the drop
    happened — fate forall over the extracted pure-lift body; the
    AS-IS `ok true` mutant skips the drop-preimages (the 0138
    leftover: ids only — the lie the real TCP plant
    `l28_real_tcp_removed_pre` refutes). -/
theorem l28_tcp_pre_ok_fate_iff :
    ∀ (b : Bool) (v : Bool),
      (l28_tcp_pre_ok b = ok v) ↔
        ((v = true ∧ b = true)
          ∨ (v = false ∧ b = false)) := by
  intro b v
  unfold l28_tcp_pre_ok
  cases b <;> cases v <;> simp

/-- RFC-0210 P1.1 (l28 cadence 4/4, atom `catalog:l28_tcp_peer`):
    after a REAL TCP plant + process death, the TCP ctor election
    timeout follows disk C-new EXACTLY when it read the disk
    membership — not the stale CLI — fate forall over the extracted
    pure-lift body; the AS-IS `ok true` mutant skips the TCP
    disk-peer timeout (the 0139 leftover: CLI n_nodes — the lie the
    real TCP plant `l28_real_tcp_removed_peer` refutes). -/
theorem l28_tcp_peer_ok_fate_iff :
    ∀ (b : Bool) (v : Bool),
      (l28_tcp_peer_ok b = ok v) ↔
        ((v = true ∧ b = true)
          ∨ (v = false ∧ b = false)) := by
  intro b v
  unfold l28_tcp_peer_ok
  cases b <;> cases v <;> simp

/-- RFC-0210 P1.1 (l28 cadence 4/4, atom `catalog:l28_tcp_lid`):
    after a REAL TCP plant + process death, the TCP ctor of a
    replica dropped from `ids` treats HashMap first-key as identity
    EXACTLY when it failed the local-id gate — fate forall over the
    extracted pure-lift body; the AS-IS `ok true` mutant skips the
    TCP local-id gate (the 0140 leftover: first-key always — the
    lie the real TCP plant `l28_real_tcp_removed_lid` refutes). -/
theorem l28_tcp_lid_ok_fate_iff :
    ∀ (b : Bool) (v : Bool),
      (l28_tcp_lid_ok b = ok v) ↔
        ((v = true ∧ b = true)
          ∨ (v = false ∧ b = false)) := by
  intro b v
  unfold l28_tcp_lid_ok
  cases b <;> cases v <;> simp

/-- RFC-0210 P1.1 (l28 cadence 4/4, atom `catalog:l28_tcp_rdr`):
    after a REAL TCP plant + process death, the TCP ctor must not
    pick the remote `ids.first()` as a LocalApplied reader
    (`empty`, not `bad node`) EXACTLY when the reader-local gate
    held — fate forall over the extracted pure-lift body; the
    AS-IS `ok true` mutant skips the TCP reader-local gate (the
    0141 leftover: ids.first always — the lie the real TCP plant
    `l28_real_tcp_removed_rdr` refutes). -/
theorem l28_tcp_rdr_ok_fate_iff :
    ∀ (b : Bool) (v : Bool),
      (l28_tcp_rdr_ok b = ok v) ↔
        ((v = true ∧ b = true)
          ∨ (v = false ∧ b = false)) := by
  intro b v
  unfold l28_tcp_rdr_ok
  cases b <;> cases v <;> simp

/-- RFC-0210 P1.1 (l28 cadence 4/4, atom `catalog:l28_tcp_dsc`):
    after a REAL TCP plant + process death, live discard drops the
    uncommitted suffix on a replica dropped from `ids` EXACTLY when
    the discard dropped it — fate forall over the extracted
    pure-lift body; the AS-IS `ok true` mutant skips live discard
    (the 0142 leftover: ids only — the lie the real TCP plant
    `l28_real_tcp_removed_dsc` refutes). -/
theorem l28_tcp_dsc_ok_fate_iff :
    ∀ (b : Bool) (v : Bool),
      (l28_tcp_dsc_ok b = ok v) ↔
        ((v = true ∧ b = true)
          ∨ (v = false ∧ b = false)) := by
  intro b v
  unfold l28_tcp_dsc_ok
  cases b <;> cases v <;> simp

/-- RFC-0210 P2.1 (l28 cadence 5/6, atom `catalog:l28_tcp_pld`):
    after a REAL TCP plant + process death, the no-leader abort
    persist-leader is local (so `next_index` repair runs) EXACTLY
    when the persist happened — fate forall over the extracted
    pure-lift body; the AS-IS `ok true` mutant skips persist-leader
    locality (the 0143 leftover: ids.first — the lie the real TCP
    plant `l28_real_tcp_removed_pld` refutes). -/
theorem l28_tcp_pld_ok_fate_iff :
    ∀ (b : Bool) (v : Bool),
      (l28_tcp_pld_ok b = ok v) ↔
        ((v = true ∧ b = true)
          ∨ (v = false ∧ b = false)) := by
  intro b v
  unfold l28_tcp_pld_ok
  cases b <;> cases v <;> simp

/-- RFC-0210 P2.1 (l28 cadence 5/6, atom `catalog:l28_tcp_std`):
    after a REAL TCP plant + process death, re-install of C-new
    steps a planted Leader down on a replica dropped from `ids`
    EXACTLY when the step-down happened — fate forall over the
    extracted pure-lift body; the AS-IS `ok true` mutant skips the
    step-down (the 0144 leftover: keep Role::Leader — the lie the
    real TCP plant `l28_real_tcp_removed_std` refutes). -/
theorem l28_tcp_std_ok_fate_iff :
    ∀ (b : Bool) (v : Bool),
      (l28_tcp_std_ok b = ok v) ↔
        ((v = true ∧ b = true)
          ∨ (v = false ∧ b = false)) := by
  intro b v
  unfold l28_tcp_std_ok
  cases b <;> cases v <;> simp

/-- RFC-0210 P2.1 (l28 cadence 5/6, atom `catalog:l28_tcp_hnt`):
    after a REAL TCP plant + process death, the TCP ctor of a
    remaining voter does not route `leader_hint` to the removed
    replica EXACTLY when the hint was filtered — fate forall over
    the extracted pure-lift body; the AS-IS `ok true` mutant skips
    the hint filter (the 0145 leftover: any leader_id — the lie the
    real TCP plant `l28_real_tcp_hint` refutes). -/
theorem l28_tcp_hnt_ok_fate_iff :
    ∀ (b : Bool) (v : Bool),
      (l28_tcp_hnt_ok b = ok v) ↔
        ((v = true ∧ b = true)
          ∨ (v = false ∧ b = false)) := by
  intro b v
  unfold l28_tcp_hnt_ok
  cases b <;> cases v <;> simp

/-- RFC-0210 P2.1 (l28 cadence 5/6, atom `catalog:l28_tcp_slot`):
    after a REAL TCP plant + process death, the TCP ctor of a
    remaining voter forgets next/match/sent_through of the removed
    replica EXACTLY when the slot was dropped — fate forall over
    the extracted pure-lift body; the AS-IS `ok true` mutant skips
    the slot drop (the 0146 leftover: keep next/match/sent_through
    — the lie the real TCP plant `l28_real_tcp_drop_repl`
    refutes). -/
theorem l28_tcp_slot_ok_fate_iff :
    ∀ (b : Bool) (v : Bool),
      (l28_tcp_slot_ok b = ok v) ↔
        ((v = true ∧ b = true)
          ∨ (v = false ∧ b = false)) := by
  intro b v
  unfold l28_tcp_slot_ok
  cases b <;> cases v <;> simp

/-- RFC-0210 P2.1 (l28 cadence 6/6, atom `catalog:l28_tcp_sth`):
    after a REAL TCP process death, the TCP ctor of a remaining
    3-node voter forgets `sent_through` of a remote replica on oob
    `remove_member` EXACTLY when the drop happened (the 0147 joint
    `drop_repl_slot` is not this tooth) — fate forall over the
    extracted pure-lift body; the AS-IS `ok true` mutant skips the
    oob sent_through drop (the 0147 leftover: keep sent_through —
    the lie the real TCP plant `l28_real_tcp_drop_st` refutes). -/
theorem l28_tcp_sth_ok_fate_iff :
    ∀ (b : Bool) (v : Bool),
      (l28_tcp_sth_ok b = ok v) ↔
        ((v = true ∧ b = true)
          ∨ (v = false ∧ b = false)) := by
  intro b v
  unfold l28_tcp_sth_ok
  cases b <;> cases v <;> simp

/-- RFC-0210 P2.1 (l28 cadence 6/6, atom `catalog:l28_tcp_pj`):
    after a REAL TCP process death, the TCP ctor of a 3-node voter
    with a planted committed C-old,new (no leave) refuses a C-old
    majority elect EXACTLY when the refusal happened — fate
    forall over the extracted pure-lift body; the AS-IS `ok true`
    mutant skips the planted joint (the 0148 leftover: elect on
    C-old — the lie the real TCP plant `l28_real_tcp_plant_joint`
    refutes). -/
theorem l28_tcp_pj_ok_fate_iff :
    ∀ (b : Bool) (v : Bool),
      (l28_tcp_pj_ok b = ok v) ↔
        ((v = true ∧ b = true)
          ∨ (v = false ∧ b = false)) := by
  intro b v
  unfold l28_tcp_pj_ok
  cases b <;> cases v <;> simp

/-- RFC-0218 P2.2 (atom `catalog:l28_durability`, entrada
    `l28_durability_ok`): a impressão digital de durabilidade é
    EXATAMENTE a conjunção citada — get_ok E after_kill_ok E restart_ok
    (cascata de ifs). O AS-IS aceita o primeiro get (o buraco 0072 —
    tooth plantado). -/
theorem l28_durability_ok_fate_iff :
    ∀ (get_ok after_kill_ok restart_ok : Bool) (v : Bool),
      (l28_durability_ok get_ok after_kill_ok restart_ok = ok v) ↔
        ((get_ok = true ∧
          ((after_kill_ok = true ∧ v = restart_ok)
           ∨ (after_kill_ok = false ∧ v = false)))
         ∨ (get_ok = false ∧ v = false)) := by
  intro get_ok after_kill_ok restart_ok v
  constructor
  · intro hval
    unfold l28_durability_ok at hval
    split at hval
    · next hg =>
      split at hval
      · next hk =>
        injection hval with hv
        exact Or.inl ⟨hg, Or.inl ⟨hk, hv.symm⟩⟩
      · next hk =>
        simp only [Bool.not_eq_true] at hk
        injection hval with hv
        exact Or.inl ⟨hg, Or.inr ⟨hk, hv.symm⟩⟩
    · next hg =>
      simp only [Bool.not_eq_true] at hg
      injection hval with hv
      exact Or.inr ⟨hg, hv.symm⟩
  · rintro (⟨rfl, (⟨rfl, rfl⟩ | ⟨rfl, rfl⟩)⟩ | ⟨rfl, rfl⟩)
    · rfl
    · rfl
    · rfl

/-- RFC-0218 P2.2 (atom `catalog:l28_napply_retry`, entrada
    `l28_tcp_napply_retry_admitted`): retries do harness NÃO são ∀
    traços TCP — admissão é a constante citada false. O AS-IS arredonda
    um napply com sucesso após >= 1 tentativa para ∀ TCP (tooth
    plantado). -/
theorem l28_tcp_napply_retry_admitted_fate_iff :
    ∀ (attempts : U64) (napply_ok : Bool) (v : Bool),
      (l28_tcp_napply_retry_admitted attempts napply_ok = ok v) ↔
        (v = false) := by
  intro attempts napply_ok v
  constructor
  · intro hval
    unfold l28_tcp_napply_retry_admitted at hval
    injection hval with hv
    exact hv.symm
  · rintro rfl
    unfold l28_tcp_napply_retry_admitted
    rfl
