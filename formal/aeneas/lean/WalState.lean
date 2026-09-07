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
