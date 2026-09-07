-- Theorems over Aeneas extract of probe_order_kernel.rs (RFC-0164).
-- Charon --start-from first_probe_on_equal_lo (walk is Iterator-refused).
import Aeneas
import ProbeOrderKernel
open Aeneas.Std Result
open pedra_aeneas_probe_order_kernel

/-- Catalog entry: equal-lo tie probes the newer table first. -/
theorem first_probe_on_equal_lo_newer :
    first_probe_on_equal_lo (1#usize) (0#usize) = ok (1#usize) := by
  unfold first_probe_on_equal_lo
  rfl

/-- AS-IS dente: equal-lo tie probes the older table first. -/
theorem first_probe_on_equal_lo_as_is_dente :
    first_probe_on_equal_lo_as_is (1#usize) (0#usize) = ok (0#usize) := by
  unfold first_probe_on_equal_lo_as_is
  rfl
