-- RFC-0199 (P1.4) → RFC-0204 (P1.1): the semantic bridges of the
-- scan/range decision. The Nat count twins and the REGISTERED bound
-- theorem (`scan_decision_work_bound`) moved to the MACHINE-EMITTED
-- `ScanGuardDerived.lean` (single emitter:
-- scripts/ratchet/derive_count_annotations.py; drift-gated by
-- lean_extracts.sh --check). What stays HERE, human by design, are
-- the bridges that pin the twins to the real Aeneas extract: an
-- overlapping file short-circuits to `ok true` without touching the
-- tombstone iterator, and every invocation of the generated closure
-- performs exactly one `tombstone_reaches_window` check and returns
-- its verdict — so per-file work is 1 + tombs, and a scan over the
-- candidate list is files + tombstones total.
import Aeneas
import ScanKernel
open Aeneas Aeneas.Std Result
open pedra_aeneas_scan_kernel

/-! ## Bridges to the real extract (human, declared) -/

/-- ok chains: a bind equal to an ok value forces the bound operation
to have returned ok (Cf.lean's `bind_ok_inv`, restated for this
module). -/
private theorem bind_ok_inv {α β} (x : Result α) (f : α → Result β) (v : β)
    (h : Aeneas.Std.bind x f = ok v) : ∃ a, x = ok a ∧ f a = ok v := by
  cases x with
  | ok a => exact ⟨a, rfl, h⟩
  | fail e => simp at h
  | div => simp at h

/-- Bridge: an overlapping file short-circuits — `scan_reads_file`
returns `ok true` without touching the tombstone iterator. -/
theorem scan_overlap_short_circuit :
    ∀ (smallest largest : Option (Slice Std.U8))
      (tombs : Slice ((Slice Std.U8) × (Slice Std.U8)))
      (start end1 : core.ops.range.Bound (Slice Std.U8)),
      scan_kernel.point_bounds_overlap smallest largest start end1 = ok true →
      scan_kernel.scan_reads_file smallest largest tombs start end1 = ok true := by
  intro smallest largest tombs start end1 hb
  unfold scan_kernel.scan_reads_file
  rw [hb]
  rfl

/-- Bridge: every invocation of the generated tombstone closure
performs exactly one `tombstone_reaches_window` check and returns its
verdict unchanged — one check per call, no extra work. (The closure
state and its argument are stated as concrete pairs: the closure's
state type is the def alias for exactly this product.) -/
theorem scan_closure_one_check_per_call :
    ∀ (start end1 : core.ops.range.Bound (Slice Std.U8))
      (t_start t_end : Slice Std.U8) (b : Bool),
      scan_kernel.scan_reads_file.closure.Insts.CoreOpsFunctionFnMutTupleSharedPairSharedSliceU8SharedSliceU8Bool.call_mut
        (start, end1) (t_start, t_end) = ok (b, (start, end1)) →
      scan_kernel.tombstone_reaches_window t_start t_end start end1 = ok b := by
  intro start end1 t_start t_end b h
  unfold
    scan_kernel.scan_reads_file.closure.Insts.CoreOpsFunctionFnMutTupleSharedPairSharedSliceU8SharedSliceU8Bool.call_mut at h
  obtain ⟨b0, hc, hres⟩ := bind_ok_inv _ _ _ h
  injection hres with hp
  injection hp with hbb _
  rw [← hbb]
  exact hc
