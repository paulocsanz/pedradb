-- Theorems over Aeneas extract of wal/wal_state_kernel.rs (RFC-0166 P1.2).
import Aeneas
import WalStateKernel
open Aeneas.Std Result
open pedra_aeneas_wal_state_kernel

/-- Catalog entry: acked ⊆ synced ⊆ written. -/
theorem inv_wal_well_formed :
    wal.wal_state_kernel.inv_wal
      { acked := 0#u64, synced := 4#u64, written := 10#u64 } = ok true := by
  unfold wal.wal_state_kernel.inv_wal
  rfl

/-- AS-IS dente: acked past the barrier still admits. -/
theorem inv_wal_as_is_dente :
    wal.wal_state_kernel.inv_wal_as_is
      { acked := 5#u64, synced := 0#u64, written := 10#u64 } = ok true := by
  unfold wal.wal_state_kernel.inv_wal_as_is
  rfl

/-- Catalog corollary: a legal cut is at/after acked.
    Unfolds `CrashModel.of` + `crash_legal`. -/
theorem acked_survives_legal_cut :
    wal.wal_state_kernel.acked_survives_every_legal_crash
      { acked := 4#u64, synced := 4#u64, written := 10#u64 }
      (7#u64) = ok true := by
  unfold wal.wal_state_kernel.acked_survives_every_legal_crash
  unfold env_crash_kernel.CrashModel.of
  unfold env_crash_kernel.crash_legal
  simp [core.cmp.Ord.min.trait_default, core.cmp.Ord.min.default,
    core.cmp.Ord.min_body, core.cmp.impls.PartialOrdU64.lt]

/-- AS-IS dente: a cut below the barrier is treated as legal and fails. -/
theorem acked_survives_as_is_dente :
    wal.wal_state_kernel.acked_survives_as_is
      { acked := 4#u64, synced := 4#u64, written := 10#u64 }
      (3#u64) = ok false := by
  unfold wal.wal_state_kernel.acked_survives_as_is
  unfold env_crash_kernel.CrashModel.of
  unfold env_crash_kernel.crash_legal_as_is
  simp [core.cmp.Ord.min.trait_default, core.cmp.Ord.min.default,
    core.cmp.Ord.min_body, core.cmp.impls.PartialOrdU64.lt]

/-- Honest `wal_sync` unfolds `env_crash_kernel.sync` and promotes. -/
theorem wal_sync_honest_promotes :
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

/-- Lying `wal_sync`: same callee, watermark stays. -/
theorem wal_sync_lying_does_not_promote :
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
