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

/-- Happy-path clock: `scale_forecast` matches `happy_get_ns`.
    Unfolds `happy_get_ns` **and** `point_get_probes`. -/
theorem happy_get_ns_l0_best_via_point_get :
    happy_get_ns 4#u64 0#u64 0#u64 = (
      do
        let i ← point_get_probes 4#u64 SCALE_L0_BEST
        let i1 ← happy_hot_bps 0#u64 0#u64
        predict_get_ns i SCALE_TAU_RAM_NS SCALE_TAU_DISK_NS i1
          SCALE_HAPPY_NOISY_BPS
    ) ∧ point_get_probes 4#u64 SCALE_L0_BEST = ok (5#u64) := by
  constructor
  · unfold happy_get_ns
    rfl
  · unfold point_get_probes
    unfold SCALE_L0_BEST
    have h : core.num.U64.saturating_add 4#u64 1#u64 = 5#u64 := by native_decide
    simp [h]

/-- AS-IS dente: walk every live file as a cold disk probe. -/
theorem happy_get_ns_as_is_dente :
    happy_get_ns_as_is 913#u64 4#u64 0#u64 0#u64 = (
      predict_get_ns_as_is 913#u64 SCALE_TAU_RAM_NS SCALE_TAU_DISK_NS 0#u64 0#u64
    ) := by
  unfold happy_get_ns_as_is
  rfl

/-- Best-path clock: `scale_forecast` matches `best_get_ns`.
    Unfolds `best_get_ns` **and** `point_get_probes`. -/
theorem best_get_ns_l0_best_via_point_get :
    best_get_ns 4#u64 = (
      do
        let i ← point_get_probes 4#u64 SCALE_L0_BEST
        predict_get_ns i SCALE_TAU_RAM_NS SCALE_TAU_DISK_NS SCALE_BPS 0#u64
    ) ∧ point_get_probes 4#u64 SCALE_L0_BEST = ok (5#u64) := by
  constructor
  · unfold best_get_ns
    rfl
  · unfold point_get_probes
    unfold SCALE_L0_BEST
    have h : core.num.U64.saturating_add 4#u64 1#u64 = 5#u64 := by native_decide
    simp [h]

/-- AS-IS dente: walk every live file as a cold disk probe. -/
theorem best_get_ns_as_is_dente :
    best_get_ns_as_is 913#u64 4#u64 = (
      predict_get_ns_as_is 913#u64 SCALE_TAU_RAM_NS SCALE_TAU_DISK_NS 0#u64 0#u64
    ) := by
  unfold best_get_ns_as_is
  rfl

/-- Empty store (keys=0): `scale_forecast` still routes best_ns through
    `best_get_ns`. Unfolds `best_get_ns` **and** `point_get_probes`. -/
theorem scale_forecast_empty_best_via_best_get_ns :
    best_get_ns 0#u64 = (
      do
        let i ← point_get_probes 0#u64 SCALE_L0_BEST
        predict_get_ns i SCALE_TAU_RAM_NS SCALE_TAU_DISK_NS SCALE_BPS 0#u64
    ) ∧ point_get_probes 0#u64 SCALE_L0_BEST = ok (1#u64) := by
  constructor
  · unfold best_get_ns
    rfl
  · unfold point_get_probes
    unfold SCALE_L0_BEST
    have h : core.num.U64.saturating_add 0#u64 1#u64 = 1#u64 := by native_decide
    simp [h]

/-- `scale_forecast` is the plan rustc links: collect levels, then the three
    clocks. Unfolds `scale_forecast` **and** `best_get_ns`. -/
theorem scale_forecast_is_three_clocks (keys ram_bytes : U64) :
    scale_forecast keys ram_bytes = (
      do
        let store_bytes ← core.num.U64.saturating_mul keys SCALE_BYTES_PER_ENTRY
        let i ← SCALE_L1_BYTES
        let i1 ← level_count store_bytes i
        let levels ← lift (core.convert.num.FromU64U32.from i1)
        let p_best ← point_get_probes levels SCALE_L0_BEST
        let p_worst ← probes_worst levels SCALE_L0_WORST
        let n_files ←
          if i = 0#u64
          then ok 0#u64
          else core.num.U64.div_ceil store_bytes i
        let warm_cap ← warm_cap_bytes ram_bytes
        let happy_hot ← happy_hot_bps store_bytes ram_bytes
        let best_ns ← best_get_ns levels
        let happy_ns ← happy_get_ns levels store_bytes ram_bytes
        let worst_ns ← worst_get_ns levels SCALE_L0_WORST
        ok
          {
            keys,
            ram_bytes,
            store_bytes,
            levels,
            p_best,
            p_worst,
            n_files,
            warm_cap,
            hot := (store_bytes <= warm_cap),
            happy_hot_bps := happy_hot,
            best_ns,
            happy_ns,
            worst_ns
          }
    ) ∧ best_get_ns 0#u64 = (
      do
        let i ← point_get_probes 0#u64 SCALE_L0_BEST
        predict_get_ns i SCALE_TAU_RAM_NS SCALE_TAU_DISK_NS SCALE_BPS 0#u64
    ) := by
  constructor
  · unfold scale_forecast
    rfl
  · unfold best_get_ns
    rfl

/-- RFC-0176 10B-key settle: 5 L1+ levels + L0-best 1 = 6 probes.
    Named test `rfc0176_one_and_ten_billion_stay_log_n`. -/
theorem rfc0176_10b_is_six_probes :
    point_get_probes 5#u64 SCALE_L0_BEST = ok (6#u64) := by
  unfold point_get_probes
  unfold SCALE_L0_BEST
  have h : core.num.U64.saturating_add 5#u64 1#u64 = 6#u64 := by native_decide
  simp [h]

/-- RFC-0176 10B-key worst production: 5 L1+ levels + L0 trigger 4 = 9. -/
theorem rfc0176_10b_worst_is_nine_probes :
    probes_worst 5#u64 4#u64 = ok (9#u64) ∧
      point_get_probes 5#u64 4#u64 = ok (9#u64) := by
  constructor
  · unfold probes_worst
    unfold point_get_probes
    have h : core.num.U64.saturating_add 5#u64 4#u64 = 9#u64 := by native_decide
    simp [h]
  · unfold point_get_probes
    have h : core.num.U64.saturating_add 5#u64 4#u64 = 9#u64 := by native_decide
    simp [h]

/-- RFC-0176 10B-key best clock: `best_get_ns` 5 levels. Unfolds
    `best_get_ns` **and** `point_get_probes`. -/
theorem rfc0176_10b_best_via_best_get_ns :
    best_get_ns 5#u64 = (
      do
        let i ← point_get_probes 5#u64 SCALE_L0_BEST
        predict_get_ns i SCALE_TAU_RAM_NS SCALE_TAU_DISK_NS SCALE_BPS 0#u64
    ) ∧ point_get_probes 5#u64 SCALE_L0_BEST = ok (6#u64) := by
  constructor
  · unfold best_get_ns
    rfl
  · unfold point_get_probes
    unfold SCALE_L0_BEST
    have h : core.num.U64.saturating_add 5#u64 1#u64 = 6#u64 := by native_decide
    simp [h]
