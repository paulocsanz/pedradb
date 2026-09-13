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

/-- Caller: missing file largest ⇒ overlap, read the file. Dual-unfold. -/
theorem scan_reads_file_none_largest
    (lo : Slice U8)
    (tombs start end1) :
    scan_kernel.scan_reads_file (some lo) none tombs start end1 = ok true := by
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

/-- Caller: unbounded end + excluded start. Dual-unfold of `scan_reads_file` and `point_bounds_overlap`. -/
theorem scan_reads_file_unbounded_end_excluded_start
    (lo hi s : Slice U8)
    (tombs : Slice ((Slice U8) × (Slice U8))) :
    scan_kernel.scan_reads_file (some lo) (some hi) tombs
      (core.ops.range.Bound.Excluded s)
      core.ops.range.Bound.Unbounded =
      (do
        let b ←
          (do
            let file_before_end ← ok true
            let file_after_start ←
              Shared1A.Insts.CoreCmpPartialOrdShared0B.gt
                (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) hi s
            if file_before_end then ok file_after_start else ok false)
        if b then ok true else
          (do
            let i ← core.slice.Slice.iter tombs
            let (b1, _) ←
              core.slice.iter.Iter.Insts.CoreIterTraitsIteratorIteratorSharedAT.any
                scan_kernel.scan_reads_file.closure.Insts.CoreOpsFunctionFnMutTupleSharedPairSharedSliceU8SharedSliceU8Bool
                i
                (core.ops.range.Bound.Excluded s, core.ops.range.Bound.Unbounded)
            ok b1)) := by
  unfold scan_kernel.scan_reads_file
  unfold scan_kernel.point_bounds_overlap
  rfl

/-- Caller: included start + included end. Dual-unfold of `scan_reads_file` and `point_bounds_overlap`. -/
theorem scan_reads_file_included_start_included_end
    (lo hi s e : Slice U8)
    (tombs : Slice ((Slice U8) × (Slice U8))) :
    scan_kernel.scan_reads_file (some lo) (some hi) tombs
      (core.ops.range.Bound.Included s)
      (core.ops.range.Bound.Included e) =
      (do
        let b ←
          (do
            let file_before_end ←
              Shared1A.Insts.CoreCmpPartialOrdShared0B.le
                (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) lo e
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
                (core.ops.range.Bound.Included s, core.ops.range.Bound.Included e)
            ok b1)) := by
  unfold scan_kernel.scan_reads_file
  unfold scan_kernel.point_bounds_overlap
  rfl

/-- Caller: excluded start + excluded end. Dual-unfold of `scan_reads_file` and `point_bounds_overlap`. -/
theorem scan_reads_file_excluded_start_excluded_end
    (lo hi s e : Slice U8)
    (tombs : Slice ((Slice U8) × (Slice U8))) :
    scan_kernel.scan_reads_file (some lo) (some hi) tombs
      (core.ops.range.Bound.Excluded s)
      (core.ops.range.Bound.Excluded e) =
      (do
        let b ←
          (do
            let file_before_end ←
              Shared1A.Insts.CoreCmpPartialOrdShared0B.lt
                (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) lo e
            let file_after_start ←
              Shared1A.Insts.CoreCmpPartialOrdShared0B.gt
                (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) hi s
            if file_before_end then ok file_after_start else ok false)
        if b then ok true else
          (do
            let i ← core.slice.Slice.iter tombs
            let (b1, _) ←
              core.slice.iter.Iter.Insts.CoreIterTraitsIteratorIteratorSharedAT.any
                scan_kernel.scan_reads_file.closure.Insts.CoreOpsFunctionFnMutTupleSharedPairSharedSliceU8SharedSliceU8Bool
                i
                (core.ops.range.Bound.Excluded s, core.ops.range.Bound.Excluded e)
            ok b1)) := by
  unfold scan_kernel.scan_reads_file
  unfold scan_kernel.point_bounds_overlap
  rfl

/-- Caller: included start + excluded end. Dual-unfold of `scan_reads_file` and `point_bounds_overlap`. -/
theorem scan_reads_file_included_start_excluded_end
    (lo hi s e : Slice U8)
    (tombs : Slice ((Slice U8) × (Slice U8))) :
    scan_kernel.scan_reads_file (some lo) (some hi) tombs
      (core.ops.range.Bound.Included s)
      (core.ops.range.Bound.Excluded e) =
      (do
        let b ←
          (do
            let file_before_end ←
              Shared1A.Insts.CoreCmpPartialOrdShared0B.lt
                (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) lo e
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
                (core.ops.range.Bound.Included s, core.ops.range.Bound.Excluded e)
            ok b1)) := by
  unfold scan_kernel.scan_reads_file
  unfold scan_kernel.point_bounds_overlap
  rfl

/-- Caller: excluded start + included end. Dual-unfold of `scan_reads_file` and `point_bounds_overlap`. -/
theorem scan_reads_file_excluded_start_included_end
    (lo hi s e : Slice U8)
    (tombs : Slice ((Slice U8) × (Slice U8))) :
    scan_kernel.scan_reads_file (some lo) (some hi) tombs
      (core.ops.range.Bound.Excluded s)
      (core.ops.range.Bound.Included e) =
      (do
        let b ←
          (do
            let file_before_end ←
              Shared1A.Insts.CoreCmpPartialOrdShared0B.le
                (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) lo e
            let file_after_start ←
              Shared1A.Insts.CoreCmpPartialOrdShared0B.gt
                (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) hi s
            if file_before_end then ok file_after_start else ok false)
        if b then ok true else
          (do
            let i ← core.slice.Slice.iter tombs
            let (b1, _) ←
              core.slice.iter.Iter.Insts.CoreIterTraitsIteratorIteratorSharedAT.any
                scan_kernel.scan_reads_file.closure.Insts.CoreOpsFunctionFnMutTupleSharedPairSharedSliceU8SharedSliceU8Bool
                i
                (core.ops.range.Bound.Excluded s, core.ops.range.Bound.Included e)
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
/-- Any ok-valued Result bind forces the bound term to be ok. -/
private theorem bind_ok_inv {α β} (x : Result α) (f : α → Result β) (v : β)
    (h : Aeneas.Std.bind x f = ok v) : ∃ a, x = ok a ∧ f a = ok v := by
  cases x with
  | ok a => exact ⟨a, rfl, h⟩
  | fail e => exact absurd h (by simp)
  | div => exact absurd h (by simp)

/-- An ok chain reassembles into an ok bind. -/
private theorem bind_intro {α β} {x : Result α} {f : α → Result β} {v : β}
    (a : α) (hx : x = ok a) (h : f a = ok v) : Aeneas.Std.bind x f = ok v := by
  rw [hx]
  exact h

/-- RFC-0218 P0.4 2/9 (átomo `catalog:sst_block_crc`): o CRC de
    bloco é EXATAMENTE a igualdade citada — casa sse stored =
    computed (sem segunda opinião). O AS-IS admite sempre (bloco
    corrompido entra — dente plantado). -/
theorem sst_block_crc_ok_fate_iff :
    ∀ (stored : U32) (computed : U32) (v : Bool),
      (scan_kernel.sst_block_crc_ok stored computed = ok v) ↔
        (v = (decide (stored = computed) : Bool)) := by
  intro stored computed v
  constructor
  · intro hval
    unfold scan_kernel.sst_block_crc_ok wal.crc.crc_match_ok at hval
    injection hval with hv
    exact hv.symm
  · rintro hv
    subst hv
    unfold scan_kernel.sst_block_crc_ok wal.crc.crc_match_ok
    rfl
/-- RFC-0218 P0.4 3/9 (átomo `catalog:zero_glue`): cola residual
    zero NUNCA é admitida — a constante citada é false (leitura
    fail-closed: sem cola não há o que ler). O AS-IS acha que a cola
    sumiu (dente plantado). -/
theorem zero_glue_admitted_fate_iff :
    ∀ (v : Bool), (scan_kernel.zero_glue_admitted = ok v) ↔ (v = false) := by
  intro v
  constructor
  · intro hval
    unfold scan_kernel.zero_glue_admitted at hval
    injection hval with hv
    exact hv.symm
  · rintro hv
    subst hv
    rfl
/-- RFC-0218 P0.4 4/9 (átomo `catalog:sst_crc`): o destino do CRC
    de SST é EXATAMENTE a árvore citada — checksum casa →
    StripTrailer; mismatch em arquivo legado (menor que o teto sem
    CRC) → WholeBuffer; mismatch moderno → Reject (fail-closed). O
    AS-IS sempre StripTrailer (trailer da sorte — dente plantado). -/
theorem sst_crc_fate_flat_fate_iff :
    ∀ (stored : U32) (computed : U32) (buf_len : Usize)
      (fate : scan_kernel.SstCrcFate),
      (scan_kernel.sst_crc_fate stored computed buf_len = ok fate) ↔
        (((decide (stored = computed) : Bool) = true ∧
            fate = scan_kernel.SstCrcFate.StripTrailer) ∨
          ((decide (stored = computed) : Bool) = false ∧
            buf_len < scan_kernel.SST_LEGACY_NO_CRC_MAX ∧
            fate = scan_kernel.SstCrcFate.WholeBuffer) ∨
          ((decide (stored = computed) : Bool) = false ∧
            ¬(buf_len < scan_kernel.SST_LEGACY_NO_CRC_MAX) ∧
            fate = scan_kernel.SstCrcFate.Reject)) := by
  intro stored computed buf_len fate
  constructor
  · intro hval
    unfold scan_kernel.sst_crc_fate at hval
    obtain ⟨b, hb, hval⟩ := bind_ok_inv _ _ _ hval
    unfold wal.crc.crc_match_ok at hb
    injection hb with hbb
    subst hbb
    split at hval
    · next hc =>
      exact Or.inl ⟨hc, by injection hval with hv; exact hv.symm⟩
    · next hc =>
      simp only [Bool.not_eq_true] at hc
      split at hval
      · next hleg =>
        exact Or.inr (Or.inl ⟨hc, hleg,
          by injection hval with hv; exact hv.symm⟩)
      · next hleg =>
        exact Or.inr (Or.inr ⟨hc, hleg,
          by injection hval with hv; exact hv.symm⟩)
  · rintro (⟨hc, hv⟩ | ⟨hc, hleg, hv⟩ | ⟨hc, hleg, hv⟩)
    · unfold scan_kernel.sst_crc_fate
      show (if (decide (stored = computed) : Bool) then
          ok scan_kernel.SstCrcFate.StripTrailer else _) = ok fate
      rw [if_pos hc]
      subst hv
      rfl
    · unfold scan_kernel.sst_crc_fate
      show (if (decide (stored = computed) : Bool) then
          ok scan_kernel.SstCrcFate.StripTrailer else _) = ok fate
      rw [if_neg (by simp only [Bool.not_eq_true]; exact hc), if_pos hleg]
      subst hv
      rfl
    · unfold scan_kernel.sst_crc_fate
      show (if (decide (stored = computed) : Bool) then
          ok scan_kernel.SstCrcFate.StripTrailer else _) = ok fate
      rw [if_neg (by simp only [Bool.not_eq_true]; exact hc), if_neg hleg]
      subst hv
      rfl

/-- Gate start of `key_in_window` (DEFEQ to the kernel's `after_start` let). -/
private noncomputable def key_in_window_after_start (key : Slice U8)
    (start : core.ops.range.Bound (Slice U8)) : Result Bool :=
  match start with
  | core.ops.range.Bound.Included s =>
    Shared1A.Insts.CoreCmpPartialOrdShared0B.ge
      (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) key s
  | core.ops.range.Bound.Excluded s =>
    Shared1A.Insts.CoreCmpPartialOrdShared0B.gt
      (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) key s
  | core.ops.range.Bound.Unbounded => ok true

/-- Gate end of `key_in_window` (DEFEQ to the kernel's `before_end` let). -/
private noncomputable def key_in_window_before_end (key : Slice U8)
    (end1 : core.ops.range.Bound (Slice U8)) : Result Bool :=
  match end1 with
  | core.ops.range.Bound.Included e =>
    Shared1A.Insts.CoreCmpPartialOrdShared0B.le
      (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) key e
  | core.ops.range.Bound.Excluded e =>
    Shared1A.Insts.CoreCmpPartialOrdShared0B.lt
      (Slice.Insts.CoreCmpPartialOrdSlice core.cmp.PartialOrdU8) key e
  | core.ops.range.Bound.Unbounded => ok true

/-- RFC-0218 P0.4 5/9 (átomo `catalog:key_in_window`): a janela
    booleana de chaves é EXATAMENTE os dois gates citados — a chave
    entra sse passou no start E passou no fim (v = a && b). O AS-IS
    só olha o start (fim da janela ignorado — dente plantado). -/
theorem key_in_window_fate_iff :
    ∀ (key : Slice U8) (start : core.ops.range.Bound (Slice U8))
      (end1 : core.ops.range.Bound (Slice U8)) (v : Bool),
      (scan_kernel.key_in_window key start end1 = ok v) ↔
        (∃ a b, key_in_window_after_start key start = ok a ∧
                key_in_window_before_end key end1 = ok b ∧
                v = (a && b)) := by
  intro key start end1 v
  constructor
  · intro hval
    unfold scan_kernel.key_in_window at hval
    obtain ⟨a, hA, hval⟩ := bind_ok_inv _ _ _ hval
    obtain ⟨b, hB, hval⟩ := bind_ok_inv _ _ _ hval
    split at hval
    · next hc =>
      rw [hc] at hA
      injection hval with hv
      exact ⟨true, b, hA, hB, hv.symm⟩
    · next hc =>
      simp only [Bool.not_eq_true] at hc
      rw [hc] at hA
      injection hval with hv
      exact ⟨false, b, hA, hB, hv.symm⟩
  · rintro ⟨a, b, hA, hB, hv⟩
    subst hv
    unfold scan_kernel.key_in_window
    exact bind_intro a hA (bind_intro b hB (by cases a <;> rfl))
