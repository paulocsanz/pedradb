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

/-- Concrete N (RFC-0176 1B-key settle): 4 L1+ levels + L0 trigger 4 = 8
    probes. Unfolds `probes_worst` **and** `point_get_probes`. -/
theorem probes_worst_l0_trigger_via_point_get :
    probes_worst 4#u64 4#u64 = ok (8#u64) ∧
      point_get_probes 4#u64 4#u64 = ok (8#u64) := by
  constructor
  · unfold probes_worst
    unfold point_get_probes
    have h : core.num.U64.saturating_add 4#u64 4#u64 = 8#u64 := by native_decide
    simp [h]
  · unfold point_get_probes
    have h : core.num.U64.saturating_add 4#u64 4#u64 = 8#u64 := by native_decide
    simp [h]

/-- Worst-path clock on that N: `scale_forecast` matches `worst_get_ns`.
    Unfolds `worst_get_ns` **and** `probes_worst`. -/
theorem worst_get_ns_l0_trigger_via_probes_worst :
    worst_get_ns 4#u64 4#u64 = (
      do
        let i ← probes_worst 4#u64 4#u64
        predict_get_ns i SCALE_TAU_RAM_NS SCALE_TAU_DISK_NS 0#u64
          SCALE_WORST_NOISY_BPS
    ) ∧ probes_worst 4#u64 4#u64 = ok (8#u64) := by
  constructor
  · unfold worst_get_ns
    rfl
  · unfold probes_worst
    unfold point_get_probes
    have h : core.num.U64.saturating_add 4#u64 4#u64 = 8#u64 := by native_decide
    simp [h]

/-- AS-IS dente: walk every live file as a cold disk probe. -/
theorem worst_get_ns_as_is_dente :
    worst_get_ns_as_is 913#u64 4#u64 4#u64 = (
      predict_get_ns_as_is 913#u64 SCALE_TAU_RAM_NS SCALE_TAU_DISK_NS 0#u64 0#u64
    ) := by
  unfold worst_get_ns_as_is
  rfl
