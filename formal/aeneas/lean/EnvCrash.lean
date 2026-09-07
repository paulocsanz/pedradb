-- Theorems over Aeneas extract of env_crash_kernel.rs (RFC-0166 P1.1).
import Aeneas
import EnvCrashKernel
open Aeneas.Std Result
open pedra_aeneas_env_crash_kernel

/-- Catalog entry: a cut between the barrier and written is legal. -/
theorem crash_legal_in_window :
    env_crash_kernel.crash_legal
      { written := 10#u64, synced := 4#u64 }
      (7#u64) = ok true := by
  unfold env_crash_kernel.crash_legal
  rfl

/-- AS-IS dente: a cut below the barrier still admits. -/
theorem crash_legal_as_is_dente :
    env_crash_kernel.crash_legal_as_is
      { written := 10#u64, synced := 4#u64 }
      (3#u64) = ok true := by
  unfold env_crash_kernel.crash_legal_as_is
  rfl
