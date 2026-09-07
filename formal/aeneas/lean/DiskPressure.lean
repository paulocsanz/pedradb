-- Theorems over Aeneas extract of disk_pressure_kernel.rs (RFC-0179).
import Aeneas
import DiskPressureKernel
open Aeneas.Std Result
open pedra_aeneas_disk_pressure_kernel

/-- Unknown probe never proactive-refuses. -/
theorem disk_pressure_unknown_admits :
    disk_pressure_admit none = ok DiskPressureAdmit.Ok := by
  unfold disk_pressure_admit
  rfl

/-- AS-IS dente: zero free still admits. -/
theorem disk_pressure_admit_as_is_dente :
    disk_pressure_admit_as_is (some 0#u64) = ok DiskPressureAdmit.Ok := by
  unfold disk_pressure_admit_as_is
  rfl
