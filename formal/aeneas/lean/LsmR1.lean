-- Theorems over Aeneas extract of lsm_r1_kernel.rs (RFC-0166 P2.1).
-- Charon --start-from catalog entries; compact nested-loop returns and
-- reopen_as_is hole patched to index loops in aeneas_lsm_r1.sh.
import Aeneas
import LsmR1Kernel
open Aeneas.Std Result
open pedra_aeneas_lsm_r1_kernel

/-- Catalog entry: reopen is identity. -/
theorem lsm_reopen_id (s) :
    lsm_reopen s = ok s := by
  unfold lsm_reopen
  rfl

/-- Catalog entry: compact of depth 0 is a no-op refuse. -/
theorem lsm_compact_depth_zero (s) :
    lsm_compact s (0#usize) = ok none := by
  unfold lsm_compact
  rfl

/-- AS-IS compact of depth 0 is also none (the mutant is the tomb drop). -/
theorem lsm_compact_as_is_depth_zero (s) :
    lsm_compact_as_is s (0#usize) = ok none := by
  unfold lsm_compact_as_is
  rfl
