-- Theorems over Aeneas extract of sst/scan_kernel.rs (RFC-0077 / F167).
-- Charon --start-from catalog entries; closure call_mut patched to
-- tombstone_reaches_window (generated Lean, restamped by aeneas_scan.sh).
import Aeneas
import ScanKernel
open Aeneas.Std Result
open pedra_aeneas_scan_kernel

/-- Catalog entry: mismatch on a modern file is Reject. -/
theorem sst_crc_fate_modern_mismatch :
    scan_kernel.sst_crc_fate (1#u32) (2#u32) (32#usize)
      = ok scan_kernel.SstCrcFate.Reject := by
  unfold scan_kernel.sst_crc_fate
  unfold wal.crc.crc_match_ok
  unfold scan_kernel.SST_LEGACY_NO_CRC_MAX
  rfl

/-- AS-IS dente: mismatch still StripTrailer. -/
theorem sst_crc_fate_as_is_dente :
    scan_kernel.sst_crc_fate_as_is (1#u32) (2#u32) (32#usize)
      = ok scan_kernel.SstCrcFate.StripTrailer := by
  unfold scan_kernel.sst_crc_fate_as_is
  rfl

/-- Catalog entry: block CRC is stored == computed. -/
theorem sst_block_crc_ok_equal :
    scan_kernel.sst_block_crc_ok (7#u32) (7#u32) = ok true := by
  unfold scan_kernel.sst_block_crc_ok
  unfold wal.crc.crc_match_ok
  rfl

/-- AS-IS dente: block mismatch still admits. -/
theorem sst_block_crc_ok_as_is_dente :
    scan_kernel.sst_block_crc_ok_as_is (1#u32) (2#u32) = ok true := by
  unfold scan_kernel.sst_block_crc_ok_as_is
  rfl

/-- Catalog entry: zero remaining glue is never admitted. -/
theorem zero_glue_admitted_false :
    scan_kernel.zero_glue_admitted = ok false := by
  unfold scan_kernel.zero_glue_admitted
  rfl

/-- AS-IS dente: glue looks gone. -/
theorem zero_glue_admitted_as_is_dente :
    scan_kernel.zero_glue_admitted_as_is = ok true := by
  unfold scan_kernel.zero_glue_admitted_as_is
  rfl

/-- Catalog entry: missing smallest bound overlaps the whole keyspace. -/
theorem scan_reads_file_none_smallest
    (largest tombs start end1) :
    scan_kernel.scan_reads_file none largest tombs start end1 = ok true := by
  unfold scan_kernel.scan_reads_file
  unfold scan_kernel.point_bounds_overlap
  rfl

/-- AS-IS dente: scan is bounds-only (tombs ignored). -/
theorem scan_reads_file_as_is_dente
    (smallest largest tombs start end1) :
    scan_kernel.scan_reads_file_as_is smallest largest tombs start end1
      = scan_kernel.point_bounds_overlap smallest largest start end1 := by
  unfold scan_kernel.scan_reads_file_as_is
  rfl

/-- Model as-is: tombstone span is ignored (always false). -/
theorem tombstone_reaches_window_as_is_dente
    (t_start t_end start end1) :
    scan_kernel.tombstone_reaches_window_as_is t_start t_end start end1
      = ok false := by
  unfold scan_kernel.tombstone_reaches_window_as_is
  rfl
