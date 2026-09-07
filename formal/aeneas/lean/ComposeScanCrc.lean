-- Cross-lib: scan_kernel.sst_crc_fate callee vs wal/crc.rs extract.
import Aeneas
import ScanKernel
import CrcKernel
open Aeneas.Std Result

/-- Equal stored/computed: both extracts of `crc_match_ok` admit. -/
theorem crc_match_ok_extracts_equal :
    pedra_aeneas_scan_kernel.wal.crc.crc_match_ok (7#u32) (7#u32)
      = pedra_aeneas_crc_kernel.crc_match_ok (7#u32) (7#u32) := by
  unfold pedra_aeneas_scan_kernel.wal.crc.crc_match_ok
  unfold pedra_aeneas_crc_kernel.crc_match_ok
  rfl

/-- Mismatch: both extracts refuse. -/
theorem crc_match_ok_extracts_mismatch :
    pedra_aeneas_scan_kernel.wal.crc.crc_match_ok (1#u32) (2#u32)
      = pedra_aeneas_crc_kernel.crc_match_ok (1#u32) (2#u32) := by
  unfold pedra_aeneas_scan_kernel.wal.crc.crc_match_ok
  unfold pedra_aeneas_crc_kernel.crc_match_ok
  rfl
