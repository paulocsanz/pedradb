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
