-- Theorems over Aeneas extract of fields_kernel.rs (RFC-0002 P24 / F60).
-- Charon --start-from catalog entries; encode_fields nested-borrows hole
-- patched to an index loop in aeneas_fields.sh.
import Aeneas
import FieldsKernel
open Aeneas.Std Result
open pedra_aeneas_fields_kernel

/-- Catalog entry: FIXED keeps the whole field. -/
theorem field_kept_id (len nul) :
    field_kept len nul = ok len := by
  unfold field_kept
  rfl

/-- AS-IS dente: keep only up to the NUL. -/
theorem field_kept_as_is_dente (len nul) :
    field_kept_as_is len nul = ok nul := by
  unfold field_kept_as_is
  rfl
