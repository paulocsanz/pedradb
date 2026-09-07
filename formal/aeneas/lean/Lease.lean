-- Theorems over Aeneas extract of dcs lease_kernel.rs (F7/F56).
-- Ord.max.default patched to pass lt, not the Ord instance.
import Aeneas
import LeaseKernel
open Aeneas.Std Result
open pedra_aeneas_lease_kernel

/-- Catalog entry: lease 0 is immortal. -/
theorem lease_live_zero :
    lease_live (0#u64) (5#u64) = ok true := by
  unfold lease_live
  rfl

/-- AS-IS dente: a past deadline still lives. -/
theorem lease_live_as_is_dente :
    lease_live_as_is (9#u64) (100#u64) = ok true := by
  unfold lease_live_as_is
  rfl
