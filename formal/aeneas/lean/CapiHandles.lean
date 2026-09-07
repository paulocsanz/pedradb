-- Theorems over Aeneas extract of capi handles.rs (RFC-0075 / F215).
-- Charon --start-from c_len_admitted (rest of handles.rs is IterMut).
import Aeneas
import CapiHandlesKernel
open Aeneas.Std Result
open pedra_aeneas_capi_handles_kernel

/-- Catalog entry: oversize len is refused. -/
theorem c_len_admitted_oversize :
    c_len_admitted (9#usize) (8#usize) = ok false := by
  unfold c_len_admitted
  rfl

/-- AS-IS dente: oversize still admits. -/
theorem c_len_admitted_as_is_dente :
    c_len_admitted_as_is (9#usize) (8#usize) = ok true := by
  unfold c_len_admitted_as_is
  rfl
