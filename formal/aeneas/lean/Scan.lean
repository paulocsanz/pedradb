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

/-- Caller: both Unbounded window, both file bounds present ⇒ overlap, read the file. Dual-unfold. -/
theorem scan_reads_file_both_unbounded
    (lo hi : Slice U8)
    (tombs : Slice ((Slice U8) × (Slice U8))) :
    scan_kernel.scan_reads_file (some lo) (some hi) tombs
      core.ops.range.Bound.Unbounded core.ops.range.Bound.Unbounded
    = ok true := by
  unfold scan_kernel.scan_reads_file
  unfold scan_kernel.point_bounds_overlap
  rfl

/-- Caller: unbounded start + included end. Dual-unfold of `scan_reads_file` and `point_bounds_overlap`. -/
theorem scan_reads_file_unbounded_start_included_end
    (lo hi e : Slice U8)
    (tombs : Slice ((Slice U8) × (Slice U8))) :
    scan_kernel.scan_reads_file (some lo) (some hi) tombs
      core.ops.range.Bound.Unbounded
      (core.ops.range.Bound.Included e) =
      (do
        let b ←
          (do
            let file_before_end ←
              Shared1A.Insts.CoreCmpPartialOrdShared0B.le
                (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) lo e
            let file_after_start ← ok true
            if file_before_end then ok file_after_start else ok false)
        if b then ok true else
          (do
            let i ← core.slice.Slice.iter tombs
            let (b1, _) ←
              core.slice.iter.Iter.Insts.CoreIterTraitsIteratorIteratorSharedAT.any
                scan_kernel.scan_reads_file.closure.Insts.CoreOpsFunctionFnMutTupleSharedPairSharedSliceU8SharedSliceU8Bool
                i
                (core.ops.range.Bound.Unbounded, core.ops.range.Bound.Included e)
            ok b1)) := by
  unfold scan_kernel.scan_reads_file
  unfold scan_kernel.point_bounds_overlap
  rfl

/-- Caller: unbounded start + excluded end. Dual-unfold of `scan_reads_file` and `point_bounds_overlap`. -/
theorem scan_reads_file_unbounded_start_excluded_end
    (lo hi e : Slice U8)
    (tombs : Slice ((Slice U8) × (Slice U8))) :
    scan_kernel.scan_reads_file (some lo) (some hi) tombs
      core.ops.range.Bound.Unbounded
      (core.ops.range.Bound.Excluded e) =
      (do
        let b ←
          (do
            let file_before_end ←
              Shared1A.Insts.CoreCmpPartialOrdShared0B.lt
                (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) lo e
            let file_after_start ← ok true
            if file_before_end then ok file_after_start else ok false)
        if b then ok true else
          (do
            let i ← core.slice.Slice.iter tombs
            let (b1, _) ←
              core.slice.iter.Iter.Insts.CoreIterTraitsIteratorIteratorSharedAT.any
                scan_kernel.scan_reads_file.closure.Insts.CoreOpsFunctionFnMutTupleSharedPairSharedSliceU8SharedSliceU8Bool
                i
                (core.ops.range.Bound.Unbounded, core.ops.range.Bound.Excluded e)
            ok b1)) := by
  unfold scan_kernel.scan_reads_file
  unfold scan_kernel.point_bounds_overlap
  rfl

/-- Caller: unbounded end + included start. Dual-unfold of `scan_reads_file` and `point_bounds_overlap`. -/
theorem scan_reads_file_unbounded_end_included_start
    (lo hi s : Slice U8)
    (tombs : Slice ((Slice U8) × (Slice U8))) :
    scan_kernel.scan_reads_file (some lo) (some hi) tombs
      (core.ops.range.Bound.Included s)
      core.ops.range.Bound.Unbounded =
      (do
        let b ←
          (do
            let file_before_end ← ok true
            let file_after_start ←
              Shared1A.Insts.CoreCmpPartialOrdShared0B.ge
                (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) hi s
            if file_before_end then ok file_after_start else ok false)
        if b then ok true else
          (do
            let i ← core.slice.Slice.iter tombs
            let (b1, _) ←
              core.slice.iter.Iter.Insts.CoreIterTraitsIteratorIteratorSharedAT.any
                scan_kernel.scan_reads_file.closure.Insts.CoreOpsFunctionFnMutTupleSharedPairSharedSliceU8SharedSliceU8Bool
                i
                (core.ops.range.Bound.Included s, core.ops.range.Bound.Unbounded)
            ok b1)) := by
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

/-- Catalog entry: missing file smallest ⇒ overlap (rustc `&[u8]` + `Bound`, not u64 cartoon). Dual-unfold. -/
theorem point_bounds_overlap_missing_smallest
    (largest : Option (Slice U8))
    (start end1 : core.ops.range.Bound (Slice U8)) :
    scan_kernel.point_bounds_overlap none largest start end1 = ok true := by
  unfold scan_kernel.point_bounds_overlap
  rfl

/-- Catalog entry: missing file largest ⇒ overlap (rustc `&[u8]` + `Bound`, not u64 cartoon). Dual-unfold. -/
theorem point_bounds_overlap_missing_largest
    (lo : Slice U8)
    (start end1 : core.ops.range.Bound (Slice U8)) :
    scan_kernel.point_bounds_overlap (some lo) none start end1 = ok true := by
  unfold scan_kernel.point_bounds_overlap
  rfl

/-- Unbounded start + excluded end: file lo `<` e, start side always true. Dual-unfold. -/
theorem point_bounds_overlap_unbounded_start_excluded_end
    (lo hi e : Slice U8) :
    scan_kernel.point_bounds_overlap (some lo) (some hi)
      core.ops.range.Bound.Unbounded
      (core.ops.range.Bound.Excluded e) =
      (do
        let file_before_end ←
          Shared1A.Insts.CoreCmpPartialOrdShared0B.lt
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) lo e
        let file_after_start ← ok true
        if file_before_end then ok file_after_start else ok false) := by
  unfold scan_kernel.point_bounds_overlap
  rfl

/-- Unbounded start + included end: file lo `<=` e, start side always true. Dual-unfold. -/
theorem point_bounds_overlap_unbounded_start_included_end
    (lo hi e : Slice U8) :
    scan_kernel.point_bounds_overlap (some lo) (some hi)
      core.ops.range.Bound.Unbounded
      (core.ops.range.Bound.Included e) =
      (do
        let file_before_end ←
          Shared1A.Insts.CoreCmpPartialOrdShared0B.le
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) lo e
        let file_after_start ← ok true
        if file_before_end then ok file_after_start else ok false) := by
  unfold scan_kernel.point_bounds_overlap
  rfl

/-- Unbounded end + excluded start: end side always true, file hi `>` s. Dual-unfold. -/
theorem point_bounds_overlap_unbounded_end_excluded_start
    (lo hi s : Slice U8) :
    scan_kernel.point_bounds_overlap (some lo) (some hi)
      (core.ops.range.Bound.Excluded s)
      core.ops.range.Bound.Unbounded =
      (do
        let file_before_end ← ok true
        let file_after_start ←
          Shared1A.Insts.CoreCmpPartialOrdShared0B.gt
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) hi s
        if file_before_end then ok file_after_start else ok false) := by
  unfold scan_kernel.point_bounds_overlap
  rfl

/-- Both Unbounded: overlap is always true (rustc `&[u8]` + `Bound`). Dual-unfold. -/
theorem point_bounds_overlap_both_unbounded
    (lo hi : Slice U8) :
    scan_kernel.point_bounds_overlap (some lo) (some hi)
      core.ops.range.Bound.Unbounded
      core.ops.range.Bound.Unbounded =
      (do
        let file_before_end ← ok true
        let file_after_start ← ok true
        if file_before_end then ok file_after_start else ok false) := by
  unfold scan_kernel.point_bounds_overlap
  rfl

/-- Included start + included end: file lo `<=` e and file hi `>=` s. Dual-unfold. -/
theorem point_bounds_overlap_included_start_included_end
    (lo hi s e : Slice U8) :
    scan_kernel.point_bounds_overlap (some lo) (some hi)
      (core.ops.range.Bound.Included s)
      (core.ops.range.Bound.Included e) =
      (do
        let file_before_end ←
          Shared1A.Insts.CoreCmpPartialOrdShared0B.le
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) lo e
        let file_after_start ←
          Shared1A.Insts.CoreCmpPartialOrdShared0B.ge
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) hi s
        if file_before_end then ok file_after_start else ok false) := by
  unfold scan_kernel.point_bounds_overlap
  rfl

/-- Excluded start + excluded end: file lo `<` e and file hi `>` s. Dual-unfold. -/
theorem point_bounds_overlap_excluded_start_excluded_end
    (lo hi s e : Slice U8) :
    scan_kernel.point_bounds_overlap (some lo) (some hi)
      (core.ops.range.Bound.Excluded s)
      (core.ops.range.Bound.Excluded e) =
      (do
        let file_before_end ←
          Shared1A.Insts.CoreCmpPartialOrdShared0B.lt
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) lo e
        let file_after_start ←
          Shared1A.Insts.CoreCmpPartialOrdShared0B.gt
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) hi s
        if file_before_end then ok file_after_start else ok false) := by
  unfold scan_kernel.point_bounds_overlap
  rfl

/-- Included start + excluded end: file lo `<` e and file hi `>=` s. Dual-unfold. -/
theorem point_bounds_overlap_included_start_excluded_end
    (lo hi s e : Slice U8) :
    scan_kernel.point_bounds_overlap (some lo) (some hi)
      (core.ops.range.Bound.Included s)
      (core.ops.range.Bound.Excluded e) =
      (do
        let file_before_end ←
          Shared1A.Insts.CoreCmpPartialOrdShared0B.lt
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) lo e
        let file_after_start ←
          Shared1A.Insts.CoreCmpPartialOrdShared0B.ge
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) hi s
        if file_before_end then ok file_after_start else ok false) := by
  unfold scan_kernel.point_bounds_overlap
  rfl

/-- Excluded start + included end: file lo `<=` e and file hi `>` s. Dual-unfold. -/
theorem point_bounds_overlap_excluded_start_included_end
    (lo hi s e : Slice U8) :
    scan_kernel.point_bounds_overlap (some lo) (some hi)
      (core.ops.range.Bound.Excluded s)
      (core.ops.range.Bound.Included e) =
      (do
        let file_before_end ←
          Shared1A.Insts.CoreCmpPartialOrdShared0B.le
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) lo e
        let file_after_start ←
          Shared1A.Insts.CoreCmpPartialOrdShared0B.gt
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) hi s
        if file_before_end then ok file_after_start else ok false) := by
  unfold scan_kernel.point_bounds_overlap
  rfl

/-- Unbounded end + included start: end side always true, file hi `>=` s. Dual-unfold. -/
theorem point_bounds_overlap_unbounded_end_included_start
    (lo hi s : Slice U8) :
    scan_kernel.point_bounds_overlap (some lo) (some hi)
      (core.ops.range.Bound.Included s)
      core.ops.range.Bound.Unbounded =
      (do
        let file_before_end ← ok true
        let file_after_start ←
          Shared1A.Insts.CoreCmpPartialOrdShared0B.ge
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) hi s
        if file_before_end then ok file_after_start else ok false) := by
  unfold scan_kernel.point_bounds_overlap
  rfl

/-- Catalog entry: unbounded window is reachable (rustc `&[u8]` + `Bound`). -/
theorem tombstone_reaches_window_unbounded (t_start t_end) :
    scan_kernel.tombstone_reaches_window t_start t_end
      core.ops.range.Bound.Unbounded core.ops.range.Bound.Unbounded
    = ok true := by
  unfold scan_kernel.tombstone_reaches_window
  rfl

/-- Exclusive window start, unbounded end: tombstone end must be `>` start. Dual-unfold. -/
theorem tombstone_reaches_window_excluded_start_unbounded_end
    (t_start t_end s) :
    scan_kernel.tombstone_reaches_window t_start t_end
      (core.ops.range.Bound.Excluded s) core.ops.range.Bound.Unbounded =
      (do
        let reaches_start ←
          Shared1A.Insts.CoreCmpPartialOrdShared0B.gt
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) t_end s
        if reaches_start then ok true else ok false) := by
  unfold scan_kernel.tombstone_reaches_window
  rfl

/-- Inclusive window start, unbounded end: rustc `Included | Excluded` both `t_end > s`. Dual-unfold. -/
theorem tombstone_reaches_window_included_start_unbounded_end
    (t_start t_end s) :
    scan_kernel.tombstone_reaches_window t_start t_end
      (core.ops.range.Bound.Included s) core.ops.range.Bound.Unbounded =
      (do
        let reaches_start ←
          Shared1A.Insts.CoreCmpPartialOrdShared0B.gt
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) t_end s
        if reaches_start then ok true else ok false) := by
  unfold scan_kernel.tombstone_reaches_window
  rfl

/-- Unbounded window start, included end: tombstone start must be `<=` e. Dual-unfold. -/
theorem tombstone_reaches_window_unbounded_start_included_end
    (t_start t_end e) :
    scan_kernel.tombstone_reaches_window t_start t_end
      core.ops.range.Bound.Unbounded
      (core.ops.range.Bound.Included e) =
      (do
        let reaches_start ← ok true
        let starts_before_end ←
          Shared1A.Insts.CoreCmpPartialOrdShared0B.le
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) t_start e
        if reaches_start then ok starts_before_end else ok false) := by
  unfold scan_kernel.tombstone_reaches_window
  rfl

/-- Unbounded window start, excluded end: tombstone start must be `<` e. Dual-unfold. -/
theorem tombstone_reaches_window_unbounded_start_excluded_end
    (t_start t_end e) :
    scan_kernel.tombstone_reaches_window t_start t_end
      core.ops.range.Bound.Unbounded
      (core.ops.range.Bound.Excluded e) =
      (do
        let reaches_start ← ok true
        let starts_before_end ←
          Shared1A.Insts.CoreCmpPartialOrdShared0B.lt
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) t_start e
        if reaches_start then ok starts_before_end else ok false) := by
  unfold scan_kernel.tombstone_reaches_window
  rfl

/-- Included start + included end: t_end `>` s and t_start `<=` e. Dual-unfold. -/
theorem tombstone_reaches_window_included_start_included_end
    (t_start t_end s e) :
    scan_kernel.tombstone_reaches_window t_start t_end
      (core.ops.range.Bound.Included s)
      (core.ops.range.Bound.Included e) =
      (do
        let reaches_start ←
          Shared1A.Insts.CoreCmpPartialOrdShared0B.gt
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) t_end s
        let starts_before_end ←
          Shared1A.Insts.CoreCmpPartialOrdShared0B.le
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) t_start e
        if reaches_start then ok starts_before_end else ok false) := by
  unfold scan_kernel.tombstone_reaches_window
  rfl

/-- Included start + excluded end: t_end `>` s and t_start `<` e. Dual-unfold. -/
theorem tombstone_reaches_window_included_start_excluded_end
    (t_start t_end s e) :
    scan_kernel.tombstone_reaches_window t_start t_end
      (core.ops.range.Bound.Included s)
      (core.ops.range.Bound.Excluded e) =
      (do
        let reaches_start ←
          Shared1A.Insts.CoreCmpPartialOrdShared0B.gt
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) t_end s
        let starts_before_end ←
          Shared1A.Insts.CoreCmpPartialOrdShared0B.lt
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) t_start e
        if reaches_start then ok starts_before_end else ok false) := by
  unfold scan_kernel.tombstone_reaches_window
  rfl

/-- Excluded start + included end: t_end `>` s and t_start `<=` e. Dual-unfold. -/
theorem tombstone_reaches_window_excluded_start_included_end
    (t_start t_end s e) :
    scan_kernel.tombstone_reaches_window t_start t_end
      (core.ops.range.Bound.Excluded s)
      (core.ops.range.Bound.Included e) =
      (do
        let reaches_start ←
          Shared1A.Insts.CoreCmpPartialOrdShared0B.gt
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) t_end s
        let starts_before_end ←
          Shared1A.Insts.CoreCmpPartialOrdShared0B.le
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) t_start e
        if reaches_start then ok starts_before_end else ok false) := by
  unfold scan_kernel.tombstone_reaches_window
  rfl

/-- Excluded start + excluded end: t_end `>` s and t_start `<` e. Dual-unfold. -/
theorem tombstone_reaches_window_excluded_start_excluded_end
    (t_start t_end s e) :
    scan_kernel.tombstone_reaches_window t_start t_end
      (core.ops.range.Bound.Excluded s)
      (core.ops.range.Bound.Excluded e) =
      (do
        let reaches_start ←
          Shared1A.Insts.CoreCmpPartialOrdShared0B.gt
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) t_end s
        let starts_before_end ←
          Shared1A.Insts.CoreCmpPartialOrdShared0B.lt
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) t_start e
        if reaches_start then ok starts_before_end else ok false) := by
  unfold scan_kernel.tombstone_reaches_window
  rfl

/-- Exclusive key window: start is slice `>` then unbounded end. Dual-unfold. -/
theorem key_in_window_excluded_unbounded (user s) :
    scan_kernel.key_in_window user (core.ops.range.Bound.Excluded s)
      core.ops.range.Bound.Unbounded =
      (do
        let after_start ←
          Shared1A.Insts.CoreCmpPartialOrdShared0B.gt
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) user s
        if after_start then ok true else ok false) := by
  unfold scan_kernel.key_in_window
  rfl

/-- Unbounded start + included end: key must be `<=` e. Dual-unfold. -/
theorem key_in_window_unbounded_start_included_end
    (user e : Slice U8) :
    scan_kernel.key_in_window user
      core.ops.range.Bound.Unbounded
      (core.ops.range.Bound.Included e) =
      (do
        let after_start ← ok true
        let before_end ←
          Shared1A.Insts.CoreCmpPartialOrdShared0B.le
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) user e
        if after_start then ok before_end else ok false) := by
  unfold scan_kernel.key_in_window
  rfl

/-- Unbounded start + excluded end: key must be `<` e. Dual-unfold. -/
theorem key_in_window_unbounded_start_excluded_end
    (user e : Slice U8) :
    scan_kernel.key_in_window user
      core.ops.range.Bound.Unbounded
      (core.ops.range.Bound.Excluded e) =
      (do
        let after_start ← ok true
        let before_end ←
          Shared1A.Insts.CoreCmpPartialOrdShared0B.lt
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) user e
        if after_start then ok before_end else ok false) := by
  unfold scan_kernel.key_in_window
  rfl

/-- Included start + included end: key `>=` s and key `<=` e. Dual-unfold. -/
theorem key_in_window_included_start_included_end
    (user s e : Slice U8) :
    scan_kernel.key_in_window user
      (core.ops.range.Bound.Included s)
      (core.ops.range.Bound.Included e) =
      (do
        let after_start ←
          Shared1A.Insts.CoreCmpPartialOrdShared0B.ge
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) user s
        let before_end ←
          Shared1A.Insts.CoreCmpPartialOrdShared0B.le
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) user e
        if after_start then ok before_end else ok false) := by
  unfold scan_kernel.key_in_window
  rfl

/-- Included start + excluded end: key `>=` s and key `<` e. Dual-unfold. -/
theorem key_in_window_included_start_excluded_end
    (user s e : Slice U8) :
    scan_kernel.key_in_window user
      (core.ops.range.Bound.Included s)
      (core.ops.range.Bound.Excluded e) =
      (do
        let after_start ←
          Shared1A.Insts.CoreCmpPartialOrdShared0B.ge
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) user s
        let before_end ←
          Shared1A.Insts.CoreCmpPartialOrdShared0B.lt
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) user e
        if after_start then ok before_end else ok false) := by
  unfold scan_kernel.key_in_window
  rfl

/-- Excluded start + excluded end: key `>` s and key `<` e. Dual-unfold. -/
theorem key_in_window_excluded_start_excluded_end
    (user s e : Slice U8) :
    scan_kernel.key_in_window user
      (core.ops.range.Bound.Excluded s)
      (core.ops.range.Bound.Excluded e) =
      (do
        let after_start ←
          Shared1A.Insts.CoreCmpPartialOrdShared0B.gt
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) user s
        let before_end ←
          Shared1A.Insts.CoreCmpPartialOrdShared0B.lt
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) user e
        if after_start then ok before_end else ok false) := by
  unfold scan_kernel.key_in_window
  rfl

/-- Excluded start + included end: key `>` s and key `<=` e. Dual-unfold. -/
theorem key_in_window_excluded_start_included_end
    (user s e : Slice U8) :
    scan_kernel.key_in_window user
      (core.ops.range.Bound.Excluded s)
      (core.ops.range.Bound.Included e) =
      (do
        let after_start ←
          Shared1A.Insts.CoreCmpPartialOrdShared0B.gt
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) user s
        let before_end ←
          Shared1A.Insts.CoreCmpPartialOrdShared0B.le
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) user e
        if after_start then ok before_end else ok false) := by
  unfold scan_kernel.key_in_window
  rfl

/-- Inclusive key window: start is slice `>=` then unbounded end. Dual-unfold. -/
theorem key_in_window_included_unbounded (user s) :
    scan_kernel.key_in_window user (core.ops.range.Bound.Included s)
      core.ops.range.Bound.Unbounded =
      (do
        let after_start ←
          Shared1A.Insts.CoreCmpPartialOrdShared0B.ge
            (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) user s
        if after_start then ok true else ok false) := by
  unfold scan_kernel.key_in_window
  rfl

/-- Catalog entry: unbounded scan window contains every key. -/
theorem key_in_window_unbounded (user) :
    scan_kernel.key_in_window user
      core.ops.range.Bound.Unbounded core.ops.range.Bound.Unbounded
    = ok true := by
  unfold scan_kernel.key_in_window
  rfl
