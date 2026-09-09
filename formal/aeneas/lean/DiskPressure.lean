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

/-- Dual-unfold: compact allowed (unknown probe) and the live reclaim plan
    recycles WAL + GC vlog + SST (RFC-0179 P1.2). -/
theorem disk_pressure_reclaim_plan_and_compact_allowed :
    compact_allowed_under_pressure none = ok true ∧
      disk_pressure_reclaim_plan true =
        ok { compact_sst := true, rotate_wal := true, compact_vlog := true } := by
  constructor
  · unfold compact_allowed_under_pressure
    rfl
  · unfold disk_pressure_reclaim_plan
    rfl

/-- AS-IS dente: SST compact only — no WAL recycle, no vlog GC. -/
theorem disk_pressure_reclaim_plan_as_is_sst_only :
    disk_pressure_reclaim_plan_as_is true =
      ok { compact_sst := true, rotate_wal := false, compact_vlog := false } := by
  unfold disk_pressure_reclaim_plan_as_is
  rfl
