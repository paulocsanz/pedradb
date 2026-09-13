-- Cross-lib: scan_kernel.sst_crc_fate (production caller) vs wal/crc.rs extract.
import Aeneas
import ScanKernel
import CrcKernel
open Aeneas.Std Result

/-- Modern mismatch: `sst_crc_fate` Rejects, and the crc extract is false. -/
theorem sst_crc_fate_mismatch_via_crc_extract :
    pedra_aeneas_scan_kernel.scan_kernel.sst_crc_fate (1#u32) (2#u32) (32#usize)
      = ok pedra_aeneas_scan_kernel.scan_kernel.SstCrcFate.Reject
    ∧ pedra_aeneas_crc_kernel.crc_match_ok (1#u32) (2#u32) = ok false := by
  constructor
  · unfold pedra_aeneas_scan_kernel.scan_kernel.sst_crc_fate
    unfold pedra_aeneas_scan_kernel.wal.crc.crc_match_ok
    unfold pedra_aeneas_scan_kernel.scan_kernel.SST_LEGACY_NO_CRC_MAX
    rfl
  · unfold pedra_aeneas_crc_kernel.crc_match_ok
    rfl

/-- Equal CRC: `sst_crc_fate` strips the trailer, and the crc extract admits. -/
theorem sst_crc_fate_equal_via_crc_extract :
    pedra_aeneas_scan_kernel.scan_kernel.sst_crc_fate (7#u32) (7#u32) (32#usize)
      = ok pedra_aeneas_scan_kernel.scan_kernel.SstCrcFate.StripTrailer
    ∧ pedra_aeneas_crc_kernel.crc_match_ok (7#u32) (7#u32) = ok true := by
  constructor
  · unfold pedra_aeneas_scan_kernel.scan_kernel.sst_crc_fate
    unfold pedra_aeneas_scan_kernel.wal.crc.crc_match_ok
    rfl
  · unfold pedra_aeneas_crc_kernel.crc_match_ok
    rfl

/-- AS-IS tooth: mismatch still StripTrailer, and the crc as-is extract admits. -/
theorem sst_crc_fate_as_is_via_crc_as_is :
    pedra_aeneas_scan_kernel.scan_kernel.sst_crc_fate_as_is (1#u32) (2#u32)
      (32#usize)
      = ok pedra_aeneas_scan_kernel.scan_kernel.SstCrcFate.StripTrailer
    ∧ pedra_aeneas_crc_kernel.crc_match_ok_as_is (1#u32) (2#u32) = ok true := by
  constructor
  · unfold pedra_aeneas_scan_kernel.scan_kernel.sst_crc_fate_as_is
    rfl
  · unfold pedra_aeneas_crc_kernel.crc_match_ok_as_is
    rfl
