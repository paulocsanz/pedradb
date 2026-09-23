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
/-- RFC-0218 P0.4 1/9 (átomo `catalog:crc_match`): a admissão de
    CRC é EXATAMENTE a igualdade citada — checksum casa sse stored =
    computed. O AS-IS admite sempre (integridade cega — dente
    plantado). -/
theorem crc_match_ok_fate_iff :
    ∀ (stored : U32) (computed : U32) (v : Bool),
      (crc_match_ok stored computed = ok v) ↔
        (v = (decide (stored = computed) : Bool)) := by
  intro stored computed v
  constructor
  · intro hval
    injection hval with hv
    exact hv.symm
  · rintro hv
    subst hv
    rfl
