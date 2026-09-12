-- Theorems over Aeneas extract of write_ack_kernel.rs (RFC-0166 P1.4).
import Aeneas
import WriteAckKernel
open Aeneas.Std Result
open pedra_aeneas_write_ack_kernel

/-- Catalog entry: on_append grows written, not the barrier. -/
theorem on_append_grows_written :
    write_ack_kernel.WriteAckLedger.on_append
      { state := { acked := 0#u64, synced := 0#u64, written := 0#u64 } }
      (96#u64) =
      ok { state := { acked := 0#u64, synced := 0#u64, written := 96#u64 } } := by
  unfold write_ack_kernel.WriteAckLedger.on_append
  unfold wal.wal_state_kernel.wal_append
  rfl

/-- AS-IS dente: ack without a barrier (acked past synced). -/
theorem write_ack_ledger_as_is_dente :
    write_ack_kernel.write_ack_ledger_as_is
      { state := { acked := 0#u64, synced := 0#u64, written := 0#u64 } }
      (96#u64) =
      ok { state := { acked := 96#u64, synced := 0#u64, written := 96#u64 } } := by
  unfold write_ack_kernel.write_ack_ledger_as_is
  unfold write_ack_kernel.WriteAckLedger.on_append
  unfold wal.wal_state_kernel.wal_append
  unfold wal.wal_state_kernel.wal_ack_as_is
  rfl

/-- Honest barrier: `on_barrier` unfolds Honest `wal_sync` + `fsync_promotes_pending`. -/
theorem on_barrier_honest_promotes :
    write_ack_kernel.WriteAckLedger.on_barrier
      { state := { acked := 0#u64, synced := 0#u64, written := 96#u64 } }
      = ok { state := { acked := 0#u64, synced := 96#u64, written := 96#u64 } } := by
  unfold write_ack_kernel.WriteAckLedger.on_barrier
  unfold wal.wal_state_kernel.wal_sync
  unfold env_crash_kernel.CrashModel.of
  unfold env_crash_kernel.sync
  unfold env_crash_kernel.SyncHonesty.Insts.CoreCmpPartialEqSyncHonesty.eq
  unfold group_commit_kernel.fsync_promotes_pending
  simp [core.cmp.Ord.min.trait_default, core.cmp.Ord.min.default,
    core.cmp.Ord.min_body, core.cmp.impls.PartialOrdU64.lt,
    env_crash_kernel.SyncHonesty.read_discriminant]

/-- Other possibility of the same caller: empty ledger, Honest promote is a no-op.
    Production `on_barrier` hardcodes Honest; there is no Lying/`on_barrier_as_is`.
    The as-is ledger path that skips the barrier is `write_ack_ledger_as_is_dente`. -/
theorem on_barrier_empty_is_id :
    write_ack_kernel.WriteAckLedger.on_barrier
      { state := { acked := 0#u64, synced := 0#u64, written := 0#u64 } }
      = ok { state := { acked := 0#u64, synced := 0#u64, written := 0#u64 } } := by
  unfold write_ack_kernel.WriteAckLedger.on_barrier
  unfold wal.wal_state_kernel.wal_sync
  unfold env_crash_kernel.CrashModel.of
  unfold env_crash_kernel.sync
  unfold env_crash_kernel.SyncHonesty.Insts.CoreCmpPartialEqSyncHonesty.eq
  unfold group_commit_kernel.fsync_promotes_pending
  simp [core.cmp.Ord.min.trait_default, core.cmp.Ord.min.default,
    core.cmp.Ord.min_body, core.cmp.impls.PartialOrdU64.lt,
    env_crash_kernel.SyncHonesty.read_discriminant]

/-- Publish: `on_ack` unfolds `wal_ack` of the synced-acked gap. -/
theorem on_ack_promotes_acked :
    write_ack_kernel.WriteAckLedger.on_ack
      { state := { acked := 0#u64, synced := 96#u64, written := 96#u64 } }
      = ok { state := { acked := 96#u64, synced := 96#u64, written := 96#u64 } } := by
  unfold write_ack_kernel.WriteAckLedger.on_ack
  have hsub : (96#u64 - 0#u64) = ok (96#u64) := rfl
  have hadd : (0#u64 + 96#u64) = ok (96#u64) := rfl
  have hsat : core.num.U64.saturating_add (0#u64) (96#u64) = 96#u64 := by
    native_decide
  have hack :
      wal.wal_state_kernel.wal_ack
        { acked := 0#u64, synced := 96#u64, written := 96#u64 } (96#u64)
        = ok { acked := 96#u64, synced := 96#u64, written := 96#u64 } := by
    unfold wal.wal_state_kernel.wal_ack
    simp [lift, hsat, hadd]
  simp [hsub, hack]

/-- Other branch of the same caller: already caught up, `wal_ack` of 0 is identity. -/
theorem on_ack_already_caught_up :
    write_ack_kernel.WriteAckLedger.on_ack
      { state := { acked := 96#u64, synced := 96#u64, written := 96#u64 } }
      = ok { state := { acked := 96#u64, synced := 96#u64, written := 96#u64 } } := by
  unfold write_ack_kernel.WriteAckLedger.on_ack
  have hsub : (96#u64 - 96#u64) = ok (0#u64) := rfl
  have hadd : (96#u64 + 0#u64) = ok (96#u64) := rfl
  have hsat : core.num.U64.saturating_add (96#u64) (0#u64) = 96#u64 := by
    native_decide
  have hack :
      wal.wal_state_kernel.wal_ack
        { acked := 96#u64, synced := 96#u64, written := 96#u64 } (0#u64)
        = ok { acked := 96#u64, synced := 96#u64, written := 96#u64 } := by
    unfold wal.wal_state_kernel.wal_ack
    simp [lift, hsat, hadd]
  simp [hsub, hack]

/-- Fail-closed Inv-WAL: `assert_inv` unfolds `inv_wal`. -/
theorem assert_inv_well_formed :
    write_ack_kernel.WriteAckLedger.assert_inv
      { state := { acked := 0#u64, synced := 4#u64, written := 10#u64 } }
      = ok () := by
  unfold write_ack_kernel.WriteAckLedger.assert_inv
  unfold wal.wal_state_kernel.inv_wal
  simp [massert]

/-- Other branch: ill-formed ledger fails the assertion. -/
theorem assert_inv_ill_formed_fails :
    write_ack_kernel.WriteAckLedger.assert_inv
      { state := { acked := 5#u64, synced := 0#u64, written := 10#u64 } }
      = fail Error.assertionFailure := by
  unfold write_ack_kernel.WriteAckLedger.assert_inv
  unfold wal.wal_state_kernel.inv_wal
  simp [massert]

/-- D1 cut inside the window: `call_mut` unfolds `d1_modelo`. -/
theorem d1_holds_cut_in_window :
    write_ack_kernel.WriteAckLedger.d1_holds_every_cut.closure.Insts.CoreOpsFunctionFnMutTupleU64Bool.call_mut
      { state := { acked := 4#u64, synced := 4#u64, written := 10#u64 } }
      (7#u64)
      = ok (true, { state := { acked := 4#u64, synced := 4#u64, written := 10#u64 } }) := by
  unfold write_ack_kernel.WriteAckLedger.d1_holds_every_cut.closure.Insts.CoreOpsFunctionFnMutTupleU64Bool.call_mut
  unfold d1_modelo_kernel.d1_modelo
  unfold wal.wal_state_kernel.inv_wal
  unfold env_crash_kernel.CrashModel.of
  unfold env_crash_kernel.crash_legal
  simp [core.cmp.Ord.min.trait_default, core.cmp.Ord.min.default,
    core.cmp.Ord.min_body, core.cmp.impls.PartialOrdU64.lt]

/-- AS-IS dente: a cut below the barrier is treated as legal and the corollary fails. -/
theorem d1_holds_as_is_cut_below_barrier :
    d1_modelo_kernel.d1_modelo_as_is
      { acked := 4#u64, synced := 4#u64, written := 10#u64 }
      (4#u64) (3#u64) = ok false := by
  unfold d1_modelo_kernel.d1_modelo_as_is
  unfold wal.wal_state_kernel.inv_wal
  unfold env_crash_kernel.CrashModel.of
  unfold env_crash_kernel.crash_legal_as_is
  simp [core.cmp.Ord.min.trait_default, core.cmp.Ord.min.default,
    core.cmp.Ord.min_body, core.cmp.impls.PartialOrdU64.lt]

/-! ## RFC-0214 P1.1 — costura WriteAck no degrau átomo (fate ∀) -/

/-- RFC-0214 P1.1 (atom `catalog:write_ack_append`): o passo append
do ledger é o átomo `wal_append` — `written` cresce por `bytes`,
`acked`/`synced` intocados (o append nunca fabrica barreira nem
ack). Fate forall sobre o corpo extraído. O mutante AS-IS
(`write_ack_ledger_as_is`) acka sem barreira — a planta DST
`verified_write_ack_on_live_profile_is_not_ok` recusa. -/
theorem on_append_fate_iff :
    ∀ (l : write_ack_kernel.WriteAckLedger) (bytes w : U64),
      (write_ack_kernel.WriteAckLedger.on_append l bytes
        = ok { l with state := { l.state with written := w } }) ↔
          (l.state.written + bytes = ok w) := by
  intro l bytes w
  constructor
  · intro h
    unfold write_ack_kernel.WriteAckLedger.on_append at h
    unfold wal.wal_state_kernel.wal_append at h
    cases hadd : l.state.written + bytes with
    | ok w' =>
        rw [hadd] at h
        simp only [bind_tc_ok] at h
        have hw : w' = w :=
          congrArg (fun s => s.state.written) (Result.ok.inj h)
        rw [hw]
    | fail e =>
        rw [hadd] at h
        simp at h
    | div =>
        rw [hadd] at h
        simp at h
  · intro h
    unfold write_ack_kernel.WriteAckLedger.on_append
    unfold wal.wal_state_kernel.wal_append
    rw [h]
    simp only [bind_tc_ok]
