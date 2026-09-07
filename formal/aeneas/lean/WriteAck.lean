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

/-- Other branch of the same callee: Lying `wal_sync` does not promote. -/
theorem wal_sync_lying_leaves_barrier :
    wal.wal_state_kernel.wal_sync
      { acked := 0#u64, synced := 0#u64, written := 96#u64 }
      env_crash_kernel.SyncHonesty.Lying
      = ok { acked := 0#u64, synced := 0#u64, written := 96#u64 } := by
  unfold wal.wal_state_kernel.wal_sync
  unfold env_crash_kernel.CrashModel.of
  unfold env_crash_kernel.sync
  unfold env_crash_kernel.SyncHonesty.Insts.CoreCmpPartialEqSyncHonesty.eq
  unfold group_commit_kernel.fsync_promotes_pending
  simp [core.cmp.Ord.min.trait_default, core.cmp.Ord.min.default,
    core.cmp.Ord.min_body, core.cmp.impls.PartialOrdU64.lt,
    env_crash_kernel.SyncHonesty.read_discriminant]
