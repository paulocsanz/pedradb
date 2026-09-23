-- Cross-lib: iter_kernel.rs and merge.rs copies of iter_window_keep.
-- Production call edge is a clone, not a shim; both extracts must agree.
import Aeneas
import IterKernel
import MergeKernel
open Aeneas.Std Result

/-- Live snapshot: both extracts keep. -/
theorem iter_merge_keep_live_agree :
    pedra_aeneas_iter_kernel.iter_window_keep true
      = pedra_aeneas_merge_kernel.merge.iter_window_keep true := by
  unfold pedra_aeneas_iter_kernel.iter_window_keep
  unfold pedra_aeneas_merge_kernel.merge.iter_window_keep
  rfl

/-- Hidden snapshot: both extracts drop. -/
theorem iter_merge_keep_hidden_agree :
    pedra_aeneas_iter_kernel.iter_window_keep false
      = pedra_aeneas_merge_kernel.merge.iter_window_keep false := by
  unfold pedra_aeneas_iter_kernel.iter_window_keep
  unfold pedra_aeneas_merge_kernel.merge.iter_window_keep
  rfl

/-- AS-IS dente: both clones emit a hidden version. -/
theorem iter_merge_keep_as_is_agree :
    pedra_aeneas_iter_kernel.iter_window_keep_as_is false
      = pedra_aeneas_merge_kernel.merge.iter_window_keep_as_is false := by
  unfold pedra_aeneas_iter_kernel.iter_window_keep_as_is
  unfold pedra_aeneas_merge_kernel.merge.iter_window_keep_as_is
  rfl
