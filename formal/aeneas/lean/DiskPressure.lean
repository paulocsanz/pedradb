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

/-- AS-IS tooth: zero free still admits. -/
theorem disk_pressure_admit_as_is_tooth :
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

/-- AS-IS tooth: SST compact only — no WAL recycle, no vlog GC. -/
theorem disk_pressure_reclaim_plan_as_is_sst_only :
    disk_pressure_reclaim_plan_as_is true =
      ok { compact_sst := true, rotate_wal := false, compact_vlog := false } := by
  unfold disk_pressure_reclaim_plan_as_is
  rfl

/-- Failed probe is unknown (`none`), never 0 free. -/
theorem disk_probe_or_unknown_err_is_none :
    disk_probe_or_unknown false none = ok none := by
  unfold disk_probe_or_unknown
  rfl

/-- AS-IS tooth: probe Err is 0 free (false-refuse). -/
theorem disk_probe_or_unknown_as_is_err_is_zero :
    disk_probe_or_unknown_as_is false none = ok (some 0#u64) := by
  unfold disk_probe_or_unknown_as_is
  rfl

/-- Dual-unfold: failed probe is unknown **and** unknown admits. -/
theorem disk_probe_err_admits :
    disk_probe_or_unknown false none = ok none ∧
      disk_pressure_admit none = ok DiskPressureAdmit.Ok := by
  constructor
  · unfold disk_probe_or_unknown
    rfl
  · unfold disk_pressure_admit
    rfl

/-- Dual-unfold: SST-write refuse (`compact_refuse`) calls
    `disk_pressure_admit`; unknown probe admits. -/
theorem compact_refuse_unfolds_disk_pressure_admit :
    compact_refuse none = ok none ∧
      disk_pressure_admit none = ok DiskPressureAdmit.Ok := by
  constructor
  · unfold compact_refuse disk_pressure_admit
    rfl
  · unfold disk_pressure_admit
    rfl

/-- AS-IS tooth: compact/flush proceeds at zero free. -/
theorem compact_refuse_as_is_tooth :
    compact_refuse_as_is (some 0#u64) = ok none := by
  unfold compact_refuse_as_is
  rfl

/-- Dual-unfold: PITR dest glue (`external_write_admitted`) calls
    `disk_pressure_admit`; unknown probe admits. -/
theorem external_write_admitted_unfolds_disk_pressure_admit :
    external_write_admitted none = ok true := by
  unfold external_write_admitted disk_pressure_admit
  rfl

/-- AS-IS tooth: dest copy/append proceeds at zero free. -/
theorem external_write_admitted_as_is_tooth :
    external_write_admitted_as_is (some 0#u64) = ok true := by
  unfold external_write_admitted_as_is
  rfl
