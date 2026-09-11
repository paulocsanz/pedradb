-- RFC-0199 (P1.4): scan/range decision work is linear.
-- Count twin of `scan_reads_file`: one bounds-overlap decision per
-- file, and only when the overlap does not short-circuit does the file
-- pay one tombstone-reach check per recorded tombstone — never a walk
-- over the file's data. The bridges pin the twins to the real Aeneas
-- extract: an overlapping file short-circuits to `ok true` without
-- touching the tombstone iterator, and every invocation of the
-- generated closure performs exactly one `tombstone_reaches_window`
-- check and returns its verdict — so per-file work is 1 + tombs, and a
-- scan over the candidate list is files + tombstones total.
import Aeneas
import ScanKernel
open Aeneas Aeneas.Std Result
open pedra_aeneas_scan_kernel

/-! ## Count twins (pure Nat) -/

/-- Work twin of the tombstone walk: one check per recorded
tombstone. -/
def scan_tomb_steps : Nat → Nat
  | 0 => 0
  | t + 1 => 1 + scan_tomb_steps t

/-- Tombstone walk bound: one step per tombstone, no more. -/
theorem scan_tomb_steps_le : ∀ (t : Nat), scan_tomb_steps t ≤ t := by
  intro t
  induction t with
  | zero => simp [scan_tomb_steps]
  | succ d ih => simp only [scan_tomb_steps]; omega

/-- A file the scan consults: whether its bounds overlap the window,
and how many tombstones it records. -/
structure ScanFile where
  overlap : Bool
  tombs : Nat

/-- Work twin of `scan_reads_file` for one file: one decision, plus
the tombstone walk only when the overlap does not short-circuit. -/
def scan_file_steps (f : ScanFile) : Nat :=
  match f.overlap with
  | true => 1
  | false => 1 + scan_tomb_steps f.tombs

/-- Work twin of the scan over the candidate file list. -/
def scan_files_steps : List ScanFile → Nat
  | [] => 0
  | f :: fs => scan_file_steps f + scan_files_steps fs

/-- Total tombstones recorded across the candidate list. -/
def scan_tomb_total : List ScanFile → Nat
  | [] => 0
  | f :: fs => f.tombs + scan_tomb_total fs

/-- RFC-0199 count (P1.4): the scan decision work over a candidate
list never exceeds one decision per file plus one check per recorded
tombstone — linear in files + tombstones, independent of file
contents. -/
theorem scan_decision_work_bound : ∀ (files : List ScanFile),
    scan_files_steps files ≤ files.length + scan_tomb_total files := by
  intro files
  induction files with
  | nil => simp [scan_files_steps, scan_tomb_total]
  | cons f fs ih =>
      have hle := scan_tomb_steps_le f.tombs
      simp only [scan_files_steps, scan_tomb_total, List.length_cons, scan_file_steps]
      split <;> omega

/-! ## Bridges to the real extract -/

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
