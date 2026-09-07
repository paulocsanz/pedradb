-- Theorems over the Aeneas extract of scale_kernel.rs (RFC-0176).
-- This is the GPS of probes, not the clock.
import Aeneas
import ScaleKernel
open Aeneas.Std Result
open pedra_aeneas_scale_kernel

/-- After settle: probes = levels + L0 covering (catalog entry). -/
theorem point_get_probes_one_plus_one :
    point_get_probes 1#u64 1#u64 = ok (2#u64) := by
  unfold point_get_probes
  have h : core.num.U64.saturating_add 1#u64 1#u64 = 2#u64 := by native_decide
  simp [h]

/-- AS-IS walk returns the file count unchanged. -/
theorem point_get_probes_as_is_is_n_files
    (n_files levels l0 : U64) :
    point_get_probes_as_is n_files levels l0 = ok n_files := by
  unfold point_get_probes_as_is
  rfl

/-- Worst-case as-is is also the file count. -/
theorem probes_worst_as_is_is_n_files
    (n_files levels l0 : U64) :
    probes_worst_as_is n_files levels l0 = ok n_files := by
  unfold probes_worst_as_is
  rfl
