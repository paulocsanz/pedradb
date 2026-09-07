-- Theorems over Aeneas extract of d1_modelo_kernel.rs (RFC-0166 P1.3).
import Aeneas
import D1ModeloKernel
open Aeneas.Std Result
open pedra_aeneas_d1_modelo_kernel

/-- Catalog entry: rec_end past acked is vacuously true. -/
theorem d1_modelo_unacked_vacuous :
    d1_modelo_kernel.d1_modelo
      { acked := 0#u64, synced := 0#u64, written := 0#u64 }
      (32#u64) (0#u64) = ok true := by
  unfold d1_modelo_kernel.d1_modelo
  unfold wal.wal_state_kernel.inv_wal
  rfl

/-- AS-IS dente: a cut below the barrier is treated as legal and the
    corollary fails (fixed kernel is vacuously true on that cut). -/
theorem d1_modelo_as_is_dente :
    d1_modelo_kernel.d1_modelo_as_is
      { acked := 10#u64, synced := 10#u64, written := 10#u64 }
      (10#u64) (3#u64) = ok false := by
  unfold d1_modelo_kernel.d1_modelo_as_is
  unfold wal.wal_state_kernel.inv_wal
  unfold env_crash_kernel.CrashModel.of
  unfold env_crash_kernel.crash_legal_as_is
  simp [core.cmp.Ord.min.trait_default, core.cmp.Ord.min.default,
    core.cmp.Ord.min_body]

/-- Honest put: append then Honest `wal_sync` then `wal_ack` of the gap. -/
theorem put_ok_append_sync_ack :
    d1_modelo_kernel.put_ok
      { acked := 0#u64, synced := 0#u64, written := 0#u64 }
      (96#u64)
      = ok { acked := 96#u64, synced := 96#u64, written := 96#u64 } := by
  unfold d1_modelo_kernel.put_ok
  have happ :
      wal.wal_state_kernel.wal_append
        { acked := 0#u64, synced := 0#u64, written := 0#u64 } (96#u64)
        = ok { acked := 0#u64, synced := 0#u64, written := 96#u64 } := by
    unfold wal.wal_state_kernel.wal_append
    rfl
  have hsync :
      wal.wal_state_kernel.wal_sync
        { acked := 0#u64, synced := 0#u64, written := 96#u64 }
        env_crash_kernel.SyncHonesty.Honest
        = ok { acked := 0#u64, synced := 96#u64, written := 96#u64 } := by
    unfold wal.wal_state_kernel.wal_sync
    unfold env_crash_kernel.CrashModel.of
    unfold env_crash_kernel.sync
    unfold env_crash_kernel.SyncHonesty.Insts.CoreCmpPartialEqSyncHonesty.eq
    unfold group_commit_kernel.fsync_promotes_pending
    simp [core.cmp.Ord.min.trait_default, core.cmp.Ord.min.default,
      core.cmp.Ord.min_body, core.cmp.impls.PartialOrdU64.lt,
      env_crash_kernel.SyncHonesty.read_discriminant]
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
  simp [happ, hsync, hsub, hack]

/-- AS-IS dente: Lying sync plus ack-past-barrier (acked > synced). -/
theorem put_ok_as_is_acks_unsynced :
    d1_modelo_kernel.put_ok_as_is
      { acked := 0#u64, synced := 0#u64, written := 0#u64 }
      (96#u64)
      = ok { acked := 96#u64, synced := 0#u64, written := 96#u64 } := by
  unfold d1_modelo_kernel.put_ok_as_is
  unfold wal.wal_state_kernel.wal_append
  unfold env_crash_kernel.CrashModel.of
  unfold env_crash_kernel.sync
  unfold env_crash_kernel.SyncHonesty.Insts.CoreCmpPartialEqSyncHonesty.eq
  unfold group_commit_kernel.fsync_promotes_pending
  simp [core.cmp.Ord.min.trait_default, core.cmp.Ord.min.default,
    core.cmp.Ord.min_body, core.cmp.impls.PartialOrdU64.lt,
    env_crash_kernel.SyncHonesty.read_discriminant]
  rfl
