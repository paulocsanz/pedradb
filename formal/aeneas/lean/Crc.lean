-- Theorems over Aeneas extract of wal/crc.rs (RFC-0076).
-- crc32c crate fns stay axioms; crc_match_ok is equality of two u32s.
import Aeneas
import CrcKernel
open Aeneas.Std Result
open pedra_aeneas_crc_kernel

/-- Catalog entry: stored == computed is Ok. -/
theorem crc_match_ok_equal :
    crc_match_ok 7#u32 7#u32 = ok true := by
  unfold crc_match_ok
  rfl

/-- AS-IS dente: mismatch still admits. -/
theorem crc_match_ok_as_is_dente :
    crc_match_ok_as_is 1#u32 2#u32 = ok true := by
  unfold crc_match_ok_as_is
  rfl
