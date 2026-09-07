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

/-- RFC-0075 P1.1: path walk is 4096 bytes. -/
theorem c_path_walk_bytes_4k :
    c_path_walk_bytes = ok (4096#usize) := by
  unfold c_path_walk_bytes
  unfold C_PATH_WALK_BYTES
  rfl

/-- AS-IS dente: walk is unbounded. -/
theorem c_path_walk_bytes_as_is_dente :
    c_path_walk_bytes_as_is = ok core.num.Usize.MAX := by
  unfold c_path_walk_bytes_as_is
  rfl

/-- Offset at the window bound is refused. -/
theorem c_path_nul_off_admitted_at_bound :
    c_path_nul_off_admitted (4096#usize) = ok false := by
  unfold c_path_nul_off_admitted
  unfold c_path_walk_bytes
  unfold C_PATH_WALK_BYTES
  simp

/-- AS-IS dente: any offset admits. -/
theorem c_path_nul_off_admitted_as_is_dente :
    c_path_nul_off_admitted_as_is (4096#usize) = ok true := by
  unfold c_path_nul_off_admitted_as_is
  rfl

/-- RFC-0075 P2.2: free table is not admitted. -/
theorem c_free_table_admitted_false :
    c_free_table_admitted = ok false := by
  unfold c_free_table_admitted
  rfl

/-- AS-IS dente: free table looks proven. -/
theorem c_free_table_admitted_as_is_dente :
    c_free_table_admitted_as_is = ok true := by
  unfold c_free_table_admitted_as_is
  rfl
